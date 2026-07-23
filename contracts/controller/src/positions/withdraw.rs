//! Withdraw flows.
//!
//! Debt-bearing accounts re-check solvency after the pool returns indexes.
//! Amount `0` means full position withdraw (mapped to `WITHDRAW_ALL_SENTINEL`).
//! Not gated by `#[when_not_paused]`: users can still exit collateral while the
//! contract is paused. Spoke-asset pause blocks withdraw; freeze does not.
//!
//! Liquidation and strategies share `settle_withdraw_entries` / `finish_withdraw_leg`
//! (see `positions` pipeline vocabulary).
//! Liquidation calls the bulk path directly and skips spoke pause enforcement.

use common::math::fp::Ray;
use common::types::{
    Account, AccountPosition, AssetConfig, HubAssetKey, PoolPositionMutation, PoolWithdrawEntry,
};
use soroban_sdk::{contractimpl, vec, Address, Env, Vec};

use crate::account::{require_owner_or_delegate, update_or_remove_supply_position};
use crate::constants::WITHDRAW_ALL_SENTINEL;
use crate::context::Cache;
use crate::events;
use crate::external::pool::pool_withdraw_call;
use crate::payments::{self, EventContext};
use crate::positions::{
    enforce_spoke_asset_flags, finalize_position_flow, get_supply_position_or_panic,
    make_pool_action, AggregatedPayments, HubPayment, PositionSides,
};
use crate::risk::{refresh_supply_risk_params, validation};
use crate::storage;
use crate::{Controller, ControllerArgs, ControllerClient};

/// Supply-risk refresh policy after a withdraw leg.
pub(crate) enum SpokeRefresh {
    /// Keep snapshotted collateral risk params (liq / delisted listing).
    Frozen,
    /// Re-stamp risk params from the account's active spoke config.
    Refresh,
}

/// Single-asset withdraw input for strategy / account-close paths.
pub(crate) struct WithdrawalRequest<'a> {
    pub hub_asset: &'a HubAssetKey,
    pub amount: i128,
    pub position: &'a AccountPosition,
}

#[contractimpl]
impl Controller {
    /// Amount `0` withdraws the full position. Returns the gross amount paid per
    /// asset (pool `actual_amount`).
    ///
    /// Not blocked by the global pause flag; spoke-asset pause still blocks the
    /// leg. Frozen assets remain withdrawable.
    ///
    /// # Arguments
    /// * `caller` - the account owner or an active delegate; must authorize.
    /// * `account_id` - existing account to withdraw from.
    /// * `withdrawals` - `(hub-asset, amount)` legs; `0` means withdraw all.
    /// * `to` - recipient of the withdrawn tokens; defaults to `caller`.
    ///
    /// # Errors
    /// * `NotAuthorized` - `caller` is neither the account owner nor an active delegate.
    /// * `FlashLoanOngoing` - a flash loan or strategy is mid-execution.
    /// * `SpokeAssetPaused` - the spoke asset is paused (frozen may still withdraw).
    /// * `CollateralPositionNotFound` - no supply position for an asset.
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

/// Auth, load account, settle, post-pool solvency, then persist supply positions.
///
/// `remove_if_empty` is true so a full exit can clean up an empty account.
/// Returned amounts are the pool's gross `actual_amount` per asset.
pub(crate) fn process_withdraw(
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
    // `zero_is_withdraw_all: true` keeps amount `0` as a full-withdraw sentinel.
    let aggregated = payments::aggregate_payments(env, withdrawals, true);

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

/// Build entries, one bulk pool withdraw, return paid amounts in input order.
fn settle_withdraw(
    env: &Env,
    account: &mut Account,
    recipient: &Address,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
) -> Vec<HubPayment> {
    let entries = build_withdraw_entries(env, account, aggregated, cache);
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

/// Per leg: spoke pause check, require supply position, map `0` → full-withdraw.
fn build_withdraw_entries(
    env: &Env,
    account: &Account,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
) -> Vec<PoolWithdrawEntry> {
    let mut entries: Vec<PoolWithdrawEntry> = Vec::new(env);
    for (hub_asset, amount) in aggregated.iter() {
        // Paused blocks withdraw; frozen still allows it.
        enforce_spoke_asset_flags(env, cache, account.spoke_id, &hub_asset, false);
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
    entries
}

/// One cross-contract pool withdraw for `entries`, then merge results input-ordered.
///
/// Does not enforce spoke pause/freeze: user and strategy paths check flags
/// before calling; liquidation calls this directly and stays exempt.
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
        finish_withdraw_leg(
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

/// Risk-param refresh policy for a withdraw leg (listing present → refresh).
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

/// Per-leg merge: scaled shares, usage, optional risk refresh, supply map, event.
///
/// Usage delta is shares withdrawn (`old_scaled - new_scaled`). Risk params are
/// refreshed only when `refresh_spoke` is `Refresh`.
pub(crate) fn finish_withdraw_leg(
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
    // Pool owns scaled shares; controller keeps collateral risk params unless refreshing.
    result_position.scaled_amount = Ray::from(result.position.scaled_amount);

    let shares_withdrawn = old_scaled.checked_sub(env, result_position.scaled_amount);
    let ctx = cache.require_spoke_usage_context(account.spoke_id);
    ctx.apply_withdraw_after_pool(env, hub_asset, shares_withdrawn);

    if matches!(refresh_spoke, SpokeRefresh::Refresh) {
        let config: AssetConfig = (&cache.require_spoke_asset(account.spoke_id, hub_asset)).into();
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
    cache.record_supply_position_update(
        action,
        hub_asset,
        result.market_index.supply_index,
        result.actual_amount,
        &result_position,
    );
}

/// Single-asset wrapper over bulk pool withdraw for strategy and account-close.
///
/// Enforces spoke pause (frozen still allowed). Liquidation bypasses this and
/// calls `settle_withdraw_entries` directly.
///
/// # Security Warning
/// * Performs no `require_auth` and re-runs no post-pool solvency gate: the
///   calling strategy entrypoint owns authorization and the final health check.
pub(crate) fn execute_withdrawal(
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
    enforce_spoke_asset_flags(env, cache, account.spoke_id, req.hub_asset, false);
    let entries = vec![
        env,
        PoolWithdrawEntry {
            action: make_pool_action(req.position, req.amount, req.hub_asset.clone()),
            protocol_fee: 0,
        },
    ];
    let results = settle_withdraw_entries(env, account, &counterparty, action, &entries, cache);
    validation::expect_invariant(env, results.get(0))
}
