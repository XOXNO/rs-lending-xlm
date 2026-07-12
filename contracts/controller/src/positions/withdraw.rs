//! Withdraw flows. Debt-bearing accounts re-run post-pool solvency gates.

use common::math::fp::Ray;
use common::types::{
    Account, AccountPosition, HubAssetKey, PoolPositionMutation, PoolWithdrawEntry,
};
use soroban_sdk::{contractimpl, Address, Env, Vec};

use crate::account::{require_owner_or_delegate, update_or_remove_supply_position};
use crate::context::Cache;
use crate::events;
use crate::external::pool::pool_withdraw_call;
use crate::payments::{self as utils, EventContext};
use crate::positions::{
    enforce_spoke_asset_flags, finalize_position_flow, get_supply_position_or_panic,
    make_pool_action, AggregatedPayments, HubPayment, PositionSides,
};
use crate::risk::refresh_supply_risk_params;
use crate::spoke;
use crate::{risk::validation, storage, Controller, ControllerArgs, ControllerClient};

/// Pool ABI sentinel for full-position withdraw (`withdraw` maps user `0` here).
pub(crate) const WITHDRAW_ALL_SENTINEL: i128 = i128::MAX;

/// Supply-risk refresh policy during withdraw.
pub(crate) enum SpokeRefresh {
    Frozen,
    Refresh,
}

/// Per-call withdrawal input.
pub(crate) struct WithdrawalRequest<'a> {
    pub hub_asset: &'a HubAssetKey,
    pub amount: i128,
    pub position: &'a AccountPosition,
}

#[contractimpl]
impl Controller {
    /// Withdraws collateral to `to` (default `caller`); an amount of `0`
    /// withdraws the full position. Returns the gross amount paid per asset.
    ///
    /// # Arguments
    /// * `caller` - the account owner or an active delegate; must authorize.
    /// * `withdrawals` - `(hub-asset, amount)` legs; `0` withdraws the full position.
    /// * `to` - recipient of the withdrawn tokens; defaults to `caller`.
    ///
    /// # Errors
    /// * `NotAuthorized` - `caller` is neither the account owner nor an active delegate.
    /// * `FlashLoanOngoing` - a flash loan or strategy is mid-execution.
    /// * `SpokeAssetPaused` - the spoke asset is paused (a frozen asset may still be withdrawn).
    /// * `CollateralPositionNotFound` - the account holds no supply position for an asset.
    /// * `InsufficientLiquidity` - the pool cannot cover the withdrawal.
    /// * Post-pool risk gates (debt-bearing accounts): `InsufficientCollateral` or
    ///   `MinBorrowCollateralNotMet`.
    ///
    /// # Events
    /// * A position-batch event summarizing the account's updated supply legs.
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

/// Withdraws collateral; amount `0` means full withdraw. Returned amounts are
/// the pool's gross actual amounts per asset.
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

    require_owner_or_delegate(env, account_id, caller, &account.owner);

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

/// Builds withdraw entries (`0` means withdraw-all), settles them, and returns
/// the gross amount paid per asset.
fn settle_withdraw(
    env: &Env,
    account: &mut Account,
    recipient: &Address,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
) -> Vec<HubPayment> {
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
    action: events::PositionAction,
    entries: &Vec<PoolWithdrawEntry>,
    cache: &mut Cache,
) -> Vec<PoolPositionMutation> {
    let is_liquidation = matches!(action, events::PositionAction::LiqSeize);
    let pool_addr = cache.cached_pool_address();
    let results = pool_withdraw_call(env, &pool_addr, recipient, is_liquidation, entries);
    for (i, entry) in entries.iter().enumerate() {
        let result = validation::expect_invariant(env, results.get(i as u32));
        let refresh_spoke = if is_liquidation {
            SpokeRefresh::Frozen
        } else {
            withdraw_refresh_spoke_for_asset(cache, account, &entry.action.hub_asset)
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

/// Params refresh while the listing exists (deprecated spokes included);
/// only removed spoke members stay frozen.
fn withdraw_refresh_spoke_for_asset(
    cache: &mut Cache,
    account: &Account,
    hub_asset: &HubAssetKey,
) -> SpokeRefresh {
    if cache
        .cached_spoke_asset(account.spoke_id, hub_asset)
        .is_none()
    {
        return SpokeRefresh::Frozen;
    }

    SpokeRefresh::Refresh
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
    result_position.scaled_amount = Ray::from(result.position.scaled_amount);
    let ctx = cache.require_spoke_usage_context(account.spoke_id);
    let delta = old_scaled - result_position.scaled_amount;
    ctx.apply_withdraw_after_pool(env, hub_asset, delta);
    // `Frozen` keeps the snapshotted params; `Refresh` re-stamps from the
    // account's active spoke config.
    if matches!(refresh_spoke, SpokeRefresh::Refresh) {
        let config = spoke::effective_asset_config(cache, account.spoke_id, hub_asset);
        refresh_supply_risk_params(
            env,
            cache,
            account,
            hub_asset,
            &mut result_position,
            &config,
        );
    }
    update_or_remove_supply_position(account, hub_asset, &result_position);

    cache.put_market_index(hub_asset, &result.market_index);
    cache.record_position_update(
        action,
        hub_asset,
        result.market_index.supply_index,
        result.actual_amount,
        &result_position,
    );
}

/// Single-asset wrapper over the bulk pool withdraw for strategy and
/// account-close paths. Enforces the per-spoke paused flag (frozen still allows
/// withdraw); liquidation bypasses this via `settle_withdraw_entries`.
///
/// # Security Warning
/// * Performs no `require_auth` and re-runs no post-pool solvency gate: the
///   calling strategy entrypoint owns authorization and the final health check.
pub fn execute_withdrawal(
    env: &Env,
    account: &mut Account,
    ctx: EventContext,
    req: WithdrawalRequest<'_>,
    cache: &mut Cache,
) -> PoolPositionMutation {
    let EventContext {
        counterparty,
        action,
    } = ctx;
    // Strategy chokepoint: paused blocks withdraw, frozen still allows it.
    // Liquidation calls `settle_withdraw_entries` directly and stays exempt.
    enforce_spoke_asset_flags(env, cache, account.spoke_id, req.hub_asset, false);
    let entries = soroban_sdk::vec![
        env,
        PoolWithdrawEntry {
            action: make_pool_action(req.position, req.amount, req.hub_asset.clone()),
            protocol_fee: 0,
        }
    ];
    let results = settle_withdraw_entries(env, account, &counterparty, action, &entries, cache);
    validation::expect_invariant(env, results.get(0))
}
