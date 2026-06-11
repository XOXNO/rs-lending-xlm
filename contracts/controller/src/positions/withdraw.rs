//! Withdraw and strategy-internal withdraw flows.
//!
//! Withdrawals re-check LTV, health factor, and touched-asset dust post-pool;
//! debt-free accounts take the `RiskDecreasing` policy and skip both gates.

use common::errors::CollateralError;
use common::math::fp::Ray;
use common::types::{
    Account, AccountPosition, EModeCategory, Payment, PoolAction, PoolPositionMutation,
    PoolWithdrawEntry,
};
use soroban_sdk::{contractimpl, panic_with_error, Address, Env, Vec};
use stellar_macros::when_not_paused;

use crate::cache::Cache;
use crate::cross_contract::pool::pool_withdraw_call;
use crate::emode;
use crate::helpers::{
    refresh_supply_risk_params, remove_account, require_no_supply_dust_for_assets,
    update_or_remove_supply_position,
};
use crate::oracle::policy::OraclePolicy;
use crate::utils::{self, EventContext};
use crate::{storage, validation, Controller, ControllerArgs, ControllerClient};

/// Pool ABI sentinel for full-position withdraw (`withdraw` maps user `0` here).
pub(crate) const WITHDRAW_ALL_SENTINEL: i128 = i128::MAX;

/// Per-call withdrawal inputs that travel together through the pipeline.
pub(crate) struct WithdrawalRequest<'a> {
    pub asset: &'a Address,
    pub amount: i128,
    pub position: &'a AccountPosition,
}

#[contractimpl]
impl Controller {
    /// Tokens go to `to` (else `caller`); returns actual paid per asset.
    #[when_not_paused]
    pub fn withdraw(
        env: Env,
        caller: Address,
        account_id: u64,
        withdrawals: Vec<(Address, i128)>,
        to: Option<Address>,
    ) -> Vec<(Address, i128)> {
        process_withdraw(&env, &caller, account_id, &withdrawals, to)
    }
}

/// Withdraws collateral and re-checks LTV, health factor, and touched-asset dust.
///
/// User amount `0` maps to the pool's `i128::MAX` full-withdraw sentinel.
pub fn process_withdraw(
    env: &Env,
    caller: &Address,
    account_id: u64,
    withdrawals: &Vec<Payment>,
    to: Option<Address>,
) -> Vec<Payment> {
    caller.require_auth();
    validation::require_not_flash_loaning(env);

    let mut account = storage::get_account(env, account_id);

    validation::require_account_owner_match(env, &account, caller);

    let recipient = to.unwrap_or_else(|| caller.clone());

    let policy = if account.borrow_positions.is_empty() {
        OraclePolicy::RiskDecreasing
    } else {
        OraclePolicy::RiskIncreasing
    };
    let mut cache = Cache::new(env, policy);

    // Aggregate once and reuse for the loop AND the post-flight dust scope.
    let plan = utils::aggregate_payments(env, withdrawals, true);

    // When the account has debt, the post-pool gates (LTV, health) price the
    // full supply+borrow set; prefetch the union here so those reads and any
    // mid-merge risk-param refresh hit the cache. When there is no debt, the
    // gates early-return and only the dust gate prices the withdrawn assets —
    // scope the prefetch to plan assets so no unread feeds are fetched.
    let dust_assets = utils::plan_assets(env, &plan);
    let priced_assets = if account.borrow_positions.is_empty() {
        dust_assets.clone()
    } else {
        let mut all = account.supply_positions.keys();
        all.append(&account.borrow_positions.keys());
        all
    };
    crate::oracle::prefetch_redstone_feeds(&mut cache, &priced_assets);

    // Build the whole plan's entries for one bulk pool call.
    let mut entries: Vec<PoolWithdrawEntry> = Vec::new(env);
    for (asset, amount) in plan.iter() {
        // `0` means withdraw all.
        let position: AccountPosition = (&account
            .supply_positions
            .get(asset.clone())
            .unwrap_or_else(|| panic_with_error!(env, CollateralError::PositionNotFound)))
            .into();
        let withdraw_amount = if amount == 0 {
            WITHDRAW_ALL_SENTINEL
        } else {
            amount
        };
        entries.push_back(PoolWithdrawEntry {
            action: PoolAction {
                position: (&position).into(),
                amount: withdraw_amount,
                asset: asset.clone(),
            },
            protocol_fee: 0,
        });
    }
    let results = settle_withdraw_entries(
        env,
        &mut account,
        &recipient,
        false,
        common::events::PositionAction::Withdraw,
        &entries,
        &mut cache,
    );

    let mut paid: Vec<Payment> = Vec::new(env);
    for (i, entry) in entries.iter().enumerate() {
        let result = validation::expect_invariant(env, results.get(i as u32));
        paid.push_back((entry.action.asset.clone(), result.actual_amount));
    }

    validation::require_within_ltv(env, &mut cache, &account);
    validation::require_healthy_account(env, &mut cache, &account);
    // Dust gate scoped to withdrawn assets only: withdraw never touches borrow
    // positions, so untouched positions that drifted under the floor must not block it.
    require_no_supply_dust_for_assets(env, &mut cache, &account, &dust_assets);

    if account.is_empty() {
        remove_account(env, account_id);
    } else {
        storage::set_supply_positions(env, account_id, &account.supply_positions);
    }
    cache.emit_position_batch(account_id, &account);
    cache.emit_market_batch();

    paid
}

/// Executes one bulk pool withdraw for `entries` (one cross-contract frame)
/// and merges the results input-ordered.
pub(crate) fn settle_withdraw_entries(
    env: &Env,
    account: &mut Account,
    recipient: &Address,
    is_liquidation: bool,
    action: common::events::PositionAction,
    entries: &Vec<PoolWithdrawEntry>,
    cache: &mut Cache,
) -> Vec<PoolPositionMutation> {
    let pool_addr = cache.cached_pool_address();
    let results = pool_withdraw_call(env, &pool_addr, recipient, is_liquidation, entries);
    // Resolve e-mode once for the whole batch; liquidation seizures never
    // refresh risk params, so they skip the read.
    let refresh_e_mode = if is_liquidation {
        None
    } else {
        Some(emode::active_e_mode_category(
            env,
            account.e_mode_category_id,
        ))
    };
    for (i, entry) in entries.iter().enumerate() {
        let result = validation::expect_invariant(env, results.get(i as u32));
        finish_withdrawal(
            env,
            account,
            action,
            &entry.action.asset,
            refresh_e_mode.as_ref(),
            &result,
            cache,
        );
    }
    results
}

/// Merges one pool withdraw result back into the account and event buffers.
/// `refresh_e_mode` is `Some` for user flows (risk params refresh from the
/// e-mode-adjusted config) and `None` for liquidation seizures (params stay
/// frozen).
pub(crate) fn finish_withdrawal(
    env: &Env,
    account: &mut Account,
    action: common::events::PositionAction,
    asset: &Address,
    refresh_e_mode: Option<&Option<EModeCategory>>,
    result: &PoolPositionMutation,
    cache: &mut Cache,
) {
    cache.record_market_update(&result.market_state);
    let mut result_position: AccountPosition = (&account
        .supply_positions
        .get(asset.clone())
        .unwrap_or_else(|| panic_with_error!(env, CollateralError::PositionNotFound)))
        .into();
    result_position.scaled_amount = Ray::from(result.position.scaled_amount_ray);
    if let Some(e_mode) = refresh_e_mode {
        let config = emode::effective_asset_config(env, account, asset, cache, e_mode);
        refresh_supply_risk_params(env, cache, account, asset, &mut result_position, &config);
    }
    update_or_remove_supply_position(account, asset, &result_position);

    cache.record_position_update(
        action,
        asset,
        result.market_index.supply_index_ray,
        result.actual_amount,
        &result_position,
    );
}

/// Single-asset wrapper over the bulk pool withdraw — used by strategies and
/// account-close paths where one asset moves per call.
pub fn execute_withdrawal(
    env: &Env,
    account: &mut Account,
    ctx: EventContext,
    req: WithdrawalRequest<'_>,
    cache: &mut Cache,
) -> PoolPositionMutation {
    let EventContext { caller, action } = ctx;
    let mut entries: Vec<PoolWithdrawEntry> = Vec::new(env);
    entries.push_back(PoolWithdrawEntry {
        action: PoolAction {
            position: req.position.into(),
            amount: req.amount,
            asset: req.asset.clone(),
        },
        protocol_fee: 0,
    });
    let results = settle_withdraw_entries(env, account, &caller, false, action, &entries, cache);
    validation::expect_invariant(env, results.get(0))
}
