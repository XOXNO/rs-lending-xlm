//! Withdraw and strategy-internal withdraw flows.
//!
//! Pipeline: auth → aggregate → cache → validate → settle → post-pool gates
//! → persist → emit. Withdrawals re-check LTV, health factor, and min-borrow
//! collateral when the account carries debt; debt-free accounts take
//! `RiskDecreasing` and skip gates.

use common::math::fp::Ray;
use controller_interface::types::{
    Account, AccountPosition, HubAssetKey, PoolPositionMutation, PoolWithdrawEntry, SpokeConfig,
};
use soroban_sdk::{contractimpl, Address, Env, Vec};

use crate::cache::Cache;
use crate::emode;
use crate::events;
use crate::external::pool::pool_withdraw_call;
use crate::helpers::utils::{self, EventContext};
use crate::helpers::{refresh_supply_risk_params, update_or_remove_supply_position};
use crate::positions::{
    enforce_spoke_asset_flags, finalize_position_flow, get_supply_position_or_panic,
    make_pool_action, AggregatedPayments, HubPayment, PositionSides,
};
use crate::{storage, validation, Controller, ControllerArgs, ControllerClient};

/// Pool ABI sentinel for full-position withdraw (`withdraw` maps user `0` here).
pub(crate) const WITHDRAW_ALL_SENTINEL: i128 = i128::MAX;

/// Per-asset decision for refreshing supply risk params during withdraw.
///
/// - `Frozen`: keep existing risk params.
/// - `None`: refresh from current config with no spoke.
/// - `From`: refresh from the given active spoke.
pub(crate) enum SpokeRefresh {
    Frozen,
    None,
    From(SpokeConfig),
}

/// Per-call withdrawal inputs that travel together through the pipeline.
pub(crate) struct WithdrawalRequest<'a> {
    pub asset: &'a Address,
    pub amount: i128,
    pub position: &'a AccountPosition,
}

#[contractimpl]
impl Controller {
    /// Tokens go to `to` (else `caller`); returns actual paid per asset.
    pub fn withdraw(
        env: Env,
        caller: Address,
        account_id: u64,
        withdrawals: Vec<(HubAssetKey, i128)>,
        to: Option<Address>,
    ) -> Vec<(HubAssetKey, i128)> {
        process_withdraw(&env, &caller, account_id, &withdrawals, to)
    }
}

/// Withdraws collateral and re-checks solvency gates when debt is present.
///
/// User amount `0` maps to the pool's `i128::MAX` full-withdraw sentinel.
pub fn process_withdraw(
    env: &Env,
    caller: &Address,
    account_id: u64,
    withdrawals: &Vec<HubPayment>,
    to: Option<Address>,
) -> Vec<HubPayment> {
    caller.require_auth();
    validation::require_not_flash_loaning(env);

    let mut account = storage::get_account(env, account_id);

    validation::require_account_owner_match(env, &account, caller);

    let recipient = to.unwrap_or_else(|| caller.clone());

    let mut cache = Cache::new(env);

    let aggregated = utils::aggregate_payments(env, withdrawals, true);
    let paid = settle_withdraw(env, &mut account, &recipient, &aggregated, &mut cache);

    validation::require_post_pool_risk_gates(env, &mut cache, &account);

    finalize_position_flow(
        env,
        account_id,
        &account,
        &mut cache,
        PositionSides::SUPPLY,
        true,
    );

    paid
}

fn settle_withdraw(
    env: &Env,
    account: &mut Account,
    recipient: &Address,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
) -> Vec<HubPayment> {
    validation::require_non_empty_payments(env, aggregated);

    let mut entries: Vec<PoolWithdrawEntry> = Vec::new(env);
    for (hub_asset, amount) in aggregated.iter() {
        // Paused blocks withdraw; frozen still allows it.
        enforce_spoke_asset_flags(env, cache, account.spoke_id, &hub_asset, false);
        // `0` means withdraw all.
        let position = get_supply_position_or_panic(env, account, &hub_asset);
        let withdraw_amount = if amount == 0 {
            WITHDRAW_ALL_SENTINEL
        } else {
            amount
        };
        entries.push_back(PoolWithdrawEntry {
            action: make_pool_action(&position, withdraw_amount, hub_asset.clone()),
            protocol_fee: 0,
        });
    }
    let results = settle_withdraw_entries(
        env,
        account,
        recipient,
        false,
        events::PositionAction::Withdraw,
        &entries,
        cache,
    );

    let mut paid: Vec<HubPayment> = Vec::new(env);
    for (i, entry) in entries.iter().enumerate() {
        let result = validation::expect_invariant(env, results.get(i as u32));
        paid.push_back((entry.action.hub_asset.clone(), result.actual_amount));
    }
    paid
}

/// Executes one bulk pool withdraw for `entries` (one cross-contract frame)
/// and merges the results input-ordered.
pub(crate) fn settle_withdraw_entries(
    env: &Env,
    account: &mut Account,
    recipient: &Address,
    is_liquidation: bool,
    action: events::PositionAction,
    entries: &Vec<PoolWithdrawEntry>,
    cache: &mut Cache,
) -> Vec<PoolPositionMutation> {
    let pool_addr = cache.cached_pool_address();
    let results = pool_withdraw_call(env, &pool_addr, recipient, is_liquidation, entries);
    // Resolve the spoke once, then decide per asset whether active membership
    // still applies.
    let spoke = if is_liquidation {
        None
    } else {
        cache.cached_spoke(account.spoke_id)
    };
    for (i, entry) in entries.iter().enumerate() {
        let result = validation::expect_invariant(env, results.get(i as u32));
        let refresh_spoke = if is_liquidation {
            SpokeRefresh::Frozen
        } else {
            withdraw_refresh_spoke_for_asset(cache, account, &entry.action.hub_asset, &spoke)
        };
        finish_withdrawal(
            env,
            account,
            action,
            &entry.action.hub_asset,
            &refresh_spoke,
            &result,
            cache,
        );
    }
    results
}

fn withdraw_refresh_spoke_for_asset(
    cache: &mut Cache,
    account: &Account,
    hub_asset: &HubAssetKey,
    spoke: &Option<SpokeConfig>,
) -> SpokeRefresh {
    if account.spoke_id == 0 {
        return SpokeRefresh::None;
    }

    let Some(spoke) = spoke else {
        return SpokeRefresh::Frozen;
    };
    if spoke.is_deprecated || cache.cached_spoke_asset(account.spoke_id, hub_asset).is_none() {
        return SpokeRefresh::Frozen;
    }

    SpokeRefresh::From(spoke.clone())
}

/// `refresh_spoke` refreshes risk params from current config or keeps them
/// frozen for liquidation, deprecated spokes, and removed spoke members.
pub(crate) fn finish_withdrawal(
    env: &Env,
    account: &mut Account,
    action: events::PositionAction,
    hub_asset: &HubAssetKey,
    refresh_spoke: &SpokeRefresh,
    result: &PoolPositionMutation,
    cache: &mut Cache,
) {
    let mut result_position = get_supply_position_or_panic(env, account, hub_asset);
    let old_scaled = result_position.scaled_amount;
    result_position.scaled_amount = Ray::from(result.position.scaled_amount_ray);
    // dimensional: scaled delta is Ray<Share(asset, supply)>.
    if let Some(ctx) = cache.spoke_usage_mut(account.spoke_id) {
        let delta = old_scaled - result_position.scaled_amount;
        ctx.apply_withdraw_after_pool(env, hub_asset, delta);
    }
    let refresh_spoke = match refresh_spoke {
        SpokeRefresh::Frozen => None,
        SpokeRefresh::None => Some(None),
        SpokeRefresh::From(spoke) => Some(Some(spoke.clone())),
    };
    if let Some(spoke) = &refresh_spoke {
        let config = emode::effective_asset_config(env, account, &hub_asset.asset, cache, spoke);
        refresh_supply_risk_params(env, cache, account, hub_asset, &mut result_position, &config);
    }
    update_or_remove_supply_position(account, hub_asset, &result_position);

    cache.put_market_index(&hub_asset.asset, &result.market_index);
    // dimensional: actual_amount is Token(asset); index is Ray<Index(asset, supply)>.
    cache.record_position_update(
        action,
        &hub_asset.asset,
        result.market_index.supply_index_ray,
        result.actual_amount,
        &result_position,
    );
}

/// Single-asset wrapper over bulk pool withdraw for strategies and account-close
/// paths where one asset moves per call.
pub fn execute_withdrawal(
    env: &Env,
    account: &mut Account,
    ctx: EventContext,
    req: WithdrawalRequest<'_>,
    cache: &mut Cache,
) -> PoolPositionMutation {
    let EventContext { caller, action } = ctx;
    let hub_asset = HubAssetKey {
        hub_id: 0,
        asset: req.asset.clone(),
    };
    let mut entries: Vec<PoolWithdrawEntry> = Vec::new(env);
    entries.push_back(PoolWithdrawEntry {
        action: make_pool_action(req.position, req.amount, hub_asset),
        protocol_fee: 0,
    });
    let results = settle_withdraw_entries(env, account, &caller, false, action, &entries, cache);
    validation::expect_invariant(env, results.get(0))
}
