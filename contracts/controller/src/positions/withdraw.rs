use common::errors::{CollateralError, GenericError};
use common::math::fp::Ray;
use common::types::{
    Account, AccountPosition, Payment, PoolAction, PoolPositionMutation, PoolWithdrawEntry,
};
use soroban_sdk::{
    assert_with_error, contractimpl, panic_with_error, Address, Env, Vec,
};
use stellar_macros::when_not_paused;

use crate::utils::EventContext;

use crate::cache::Cache;
use crate::cross_contract::pool::pool_withdraw_call;
use crate::helpers::{
    refresh_supply_risk_params_for_asset, require_no_supply_dust_for_assets,
    update_or_remove_supply_position,
};
use crate::oracle::policy::OraclePolicy;
use crate::{storage, utils::*, validation, Controller, ControllerArgs, ControllerClient};

/// Pool ABI sentinel for full-position withdraw (`withdraw` maps user `0` here).
pub(crate) const WITHDRAW_ALL_SENTINEL: i128 = i128::MAX;

/// Per-call withdrawal inputs that travel together through the pipeline.
pub(crate) struct WithdrawalRequest<'a> {
    pub asset: &'a Address,
    pub amount: i128,
    pub position: &'a AccountPosition,
}

/// Liquidation-only modifiers; default is a plain withdraw.
#[derive(Clone, Copy)]
pub(crate) struct WithdrawFlags {
    pub is_liquidation: bool,
    pub protocol_fee: i128,
}

impl WithdrawFlags {
    pub fn plain() -> Self {
        Self {
            is_liquidation: false,
            protocol_fee: 0,
        }
    }
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
    let withdrawal_plan = aggregate_payments(env, withdrawals, true);

    // When the account has debt, the withdrawal loop prices the withdrawn
    // asset, then require_within_ltv and require_healthy_account price the
    // full supply+borrow set — all before any downstream prefetch can fire
    // with the complete set.  Collect supply+borrow keys and bulk-prefetch
    // them so every price read hits the cache instead of single-resolving.
    //
    // When there is no debt, require_within_ltv and require_healthy_account
    // both early-return without pricing anything; only the withdrawn asset(s)
    // are priced by the withdrawal loop and the dust gate.  Prefetching the
    // full supply set in that case would fire a bulk call for non-plan feeds
    // that are never read — so scope the prefetch to plan assets only.
    // (The dust gate re-prices nothing: the withdrawal loop's price reads land
    // in the shared tx-local prices_cache, so the gate's own prefetch and
    // reads are cache hits.)
    let dust_assets = plan_assets(env, &withdrawal_plan);
    let priced_assets = if account.borrow_positions.is_empty() {
        dust_assets.clone()
    } else {
        let mut all = account.supply_positions.keys();
        all.append(&account.borrow_positions.keys());
        all
    };
    crate::oracle::prefetch_redstone_feeds(&mut cache, &priced_assets);

    // Build the whole plan's entries, make ONE pool call, then merge results
    // input-ordered — one cross-contract frame instead of one per asset.
    let mut entries: Vec<PoolWithdrawEntry> = Vec::new(env);
    for (asset, amount) in withdrawal_plan.iter() {
        // `0` means withdraw all; negative withdrawals are never valid.
        assert_with_error!(env, amount >= 0, GenericError::AmountMustBePositive);
        let position: AccountPosition = match account.supply_positions.get(asset.clone()) {
            Some(pos) => (&pos).into(),
            None => panic_with_error!(env, CollateralError::PositionNotFound),
        };
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
    let pool_addr = cache.cached_pool_address();
    let results = pool_withdraw_call(env, &pool_addr, &recipient, false, &entries);

    let mut paid: Vec<Payment> = Vec::new(env);
    for (i, entry) in entries.iter().enumerate() {
        let result = validation::expect_invariant(env, results.get(i as u32));
        finish_withdrawal(
            env,
            &mut account,
            common::events::PositionAction::Withdraw,
            &entry.action.asset,
            false,
            &result,
            &mut cache,
        );
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
        // Mutates supply positions only.
        storage::set_supply_positions(env, account_id, &account.supply_positions);
    }
    cache.emit_position_batch(account_id, &account);
    cache.emit_market_batch();

    paid
}

/// Single-asset wrapper over the bulk pool withdraw — used by strategies and
/// account-close paths where one asset moves per call (bulk-of-one costs the
/// same frame as the old single endpoint).
pub fn execute_withdrawal(
    env: &Env,
    account: &mut Account,
    ctx: EventContext,
    req: WithdrawalRequest<'_>,
    flags: WithdrawFlags,
    cache: &mut Cache,
) -> PoolPositionMutation {
    let EventContext { caller, action } = ctx;
    let pool_addr = cache.cached_pool_address();
    let mut entries: Vec<PoolWithdrawEntry> = Vec::new(env);
    entries.push_back(PoolWithdrawEntry {
        action: PoolAction {
            position: req.position.into(),
            amount: req.amount,
            asset: req.asset.clone(),
        },
        protocol_fee: flags.protocol_fee,
    });
    let results = pool_withdraw_call(env, &pool_addr, &caller, flags.is_liquidation, &entries);
    let result = validation::expect_invariant(env, results.get(0));
    finish_withdrawal(
        env,
        account,
        action,
        req.asset,
        flags.is_liquidation,
        &result,
        cache,
    );
    result
}

/// Merges one pool withdraw result back into the account and event buffers.
pub(crate) fn finish_withdrawal(
    env: &Env,
    account: &mut Account,
    action: common::events::PositionAction,
    asset: &Address,
    is_liquidation: bool,
    result: &PoolPositionMutation,
    cache: &mut Cache,
) {
    cache.record_market_update(&result.market_state);
    let mut result_position: AccountPosition = match account.supply_positions.get(asset.clone()) {
        Some(pos) => (&pos).into(),
        None => panic_with_error!(env, CollateralError::PositionNotFound),
    };
    result_position.scaled_amount = Ray::from(result.position.scaled_amount_ray);
    if !is_liquidation {
        refresh_supply_risk_params_for_asset(env, cache, account, asset, &mut result_position);
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
