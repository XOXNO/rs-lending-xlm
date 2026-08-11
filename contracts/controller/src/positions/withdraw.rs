//! Withdraw-flow logic for the controller: transfers withdrawn funds from the pool, applies the
//! pool results to account supply positions, refreshes risk parameters after the balance
//! shrinks, and re-runs solvency checks. Also exposes a single-leg withdraw entry point used by
//! liquidation.

use common::math::fp::Ray;
use common::types::{
    Account, AccountPosition, AssetConfig, HubAssetKey, PoolPositionMutation, PoolWithdrawEntry,
};
use soroban_sdk::{vec, Address, Env, Vec};

use crate::account::{require_owner_or_delegate, update_or_remove_supply_position};
use crate::constants::WITHDRAW_ALL_SENTINEL;
use crate::context::Cache;
use crate::events;
use crate::events::EventContext;
use crate::external::pool::pool_withdraw_call;
use crate::payments;
use crate::positions::{
    apply_leg_usage, enforce_post_pool_solvency, enforce_spoke_asset_flags, finalize_position_flow,
    for_each_leg, get_supply_position_or_panic, make_pool_action, require_position_caller,
    AggregatedPayments, FreezePolicy, HubPayment, LegDirection, LegOutcome, PositionSides,
};
use crate::risk::{refresh_supply_risk_params, RiskRefreshScope};
use crate::spoke::UsageSide;
use crate::storage;
use common::validation::expect_invariant;

/// Whether a withdraw batch is a normal user withdrawal or part of a liquidation.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum WithdrawKind {
    Normal,

    Liquidation,
}

/// Whether [`merge_withdraw_leg`] refreshes the position's risk parameters after applying the
/// withdrawal.
pub(crate) enum SpokeRefresh {
    Frozen,

    Refresh,
}

/// A single supply leg to withdraw: the hub asset, the amount to withdraw, and the current
/// supply position.
pub(crate) struct WithdrawalRequest<'a> {
    pub hub_asset: &'a HubAssetKey,
    pub amount: i128,
    pub position: &'a AccountPosition,
}

/// Executes a withdrawal of `withdrawals` from `account_id`'s supply positions, sending the
/// withdrawn funds to `to` (or `caller` if `to` is `None`). A zero amount for a hub asset means
/// withdraw all. Returns the actual amount paid per hub asset.
///
/// Requires `caller`'s authorization and reverts if a flash loan is in progress, and requires
/// `caller` to be the account owner or an active protocol position manager listed among the
/// account's delegates. Re-runs solvency checks after the
/// pool call and removes the account from storage if the withdrawal leaves it empty.
pub(crate) fn process_withdraw(
    env: &Env,
    caller: &Address,
    account_id: u64,
    withdrawals: &Vec<HubPayment>,
    to: Option<Address>,
) -> Vec<HubPayment> {
    require_position_caller(env, caller);

    let mut account = storage::get_account(env, account_id);
    require_owner_or_delegate(env, account_id, caller, &account.owner);

    let recipient = to.unwrap_or_else(|| caller.clone());
    let mut cache = Cache::new(env);

    let aggregated = payments::aggregate_payments(env, withdrawals, payments::ZeroLeg::MeansAll);

    let paid = settle_withdraw(env, &mut account, &recipient, &aggregated, &mut cache);

    let _ = enforce_post_pool_solvency(env, &mut cache, &mut account);

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

/// Builds pool withdraw entries for `aggregated`, applies them against the pool crediting
/// `recipient`, and updates `account`'s supply positions from the results. Returns the actual
/// amount paid per hub asset.
fn settle_withdraw(
    env: &Env,
    account: &mut Account,
    recipient: &Address,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
) -> Vec<HubPayment> {
    let entries = build_withdraw_entries(env, account, aggregated, cache);
    let results = apply_withdraw_batch(
        env,
        account,
        recipient,
        WithdrawKind::Normal,
        events::PositionAction::Withdraw,
        &entries,
        cache,
    );

    let mut paid: Vec<HubPayment> = Vec::new(env);
    for (entry, result) in entries.iter().zip(results.iter()) {
        paid.push_back((entry.action.hub_asset, result.actual_amount));
    }
    paid
}

/// Builds one [`PoolWithdrawEntry`] per hub asset in `aggregated`: enforces the asset's
/// pause/freeze flags for an exit and resolves a zero amount to the withdraw-all sentinel.
///
/// Panics with `CollateralPositionNotFound` if a hub asset in `aggregated` has no existing
/// supply position on `account`.
fn build_withdraw_entries(
    env: &Env,
    account: &Account,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
) -> Vec<PoolWithdrawEntry> {
    let mut entries: Vec<PoolWithdrawEntry> = Vec::new(env);
    for (hub_asset, amount) in aggregated.iter() {
        enforce_spoke_asset_flags(
            env,
            cache,
            account.spoke_id,
            &hub_asset,
            FreezePolicy::AllowOnExit,
        );
        let position = get_supply_position_or_panic(env, account, &hub_asset);
        entries.push_back(PoolWithdrawEntry {
            action: make_pool_action(
                &position,
                resolve_withdraw_amount(amount),
                hub_asset.clone(),
            ),
            protocol_fee: 0,
        });
    }
    entries
}

/// Returns [`WITHDRAW_ALL_SENTINEL`] if `amount` is zero, otherwise returns `amount` unchanged.
fn resolve_withdraw_amount(amount: i128) -> i128 {
    if amount == 0 {
        WITHDRAW_ALL_SENTINEL
    } else {
        amount
    }
}

/// Withdraws the single supply leg described by `req` against `account`, using `ctx`'s
/// counterparty as recipient and `ctx`'s action as the recorded event action.
///
/// Enforces the asset's pause/freeze flags for an exit, then applies the withdrawal as a
/// [`WithdrawKind::Normal`] batch through [`apply_withdraw_batch`]. Panics via `expect_invariant`
/// if the pool returns no result for the withdrawal.
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
    enforce_spoke_asset_flags(
        env,
        cache,
        account.spoke_id,
        req.hub_asset,
        FreezePolicy::AllowOnExit,
    );
    let entries = vec![
        env,
        PoolWithdrawEntry {
            action: make_pool_action(req.position, req.amount, req.hub_asset.clone()),
            protocol_fee: 0,
        },
    ];
    let results = apply_withdraw_batch(
        env,
        account,
        &counterparty,
        WithdrawKind::Normal,
        action,
        &entries,
        cache,
    );
    expect_invariant(env, results.get(0))
}

/// Calls the pool to execute `entries` as a withdraw batch crediting `recipient`, passing
/// `kind == WithdrawKind::Liquidation` through to the pool call, then merges each leg's result
/// into `account`'s supply positions, recording `action` as the event action. Returns the pool's
/// per-leg mutation results.
pub(crate) fn apply_withdraw_batch(
    env: &Env,
    account: &mut Account,
    recipient: &Address,
    kind: WithdrawKind,
    action: events::PositionAction,
    entries: &Vec<PoolWithdrawEntry>,
    cache: &mut Cache,
) -> Vec<PoolPositionMutation> {
    let is_liquidation = kind == WithdrawKind::Liquidation;
    let pool_addr = cache.cached_pool_address();
    let results = pool_withdraw_call(env, &pool_addr, recipient, is_liquidation, entries);
    for_each_leg(env, entries, &results, |entry, result| {
        let hub_asset = entry.action.hub_asset;
        let outcome = LegOutcome::from(&result);
        let refresh_spoke =
            spoke_refresh_for_leg(kind, cache, account, &hub_asset, outcome.new_scaled);
        merge_withdraw_leg(
            env,
            account,
            action,
            &hub_asset,
            &refresh_spoke,
            &outcome,
            cache,
        );
    });
    results
}

/// Folds one supply-side pool result into `account`: applies the new scaled amount, updates
/// spoke usage and the cached market index, refreshes the position's risk parameters when
/// `refresh_spoke` is [`SpokeRefresh::Refresh`] and the resulting balance is nonzero, and records
/// the supply position update event.
///
/// Panics with `CollateralPositionNotFound` if `hub_asset` has no existing supply position on
/// `account`.
pub(crate) fn merge_withdraw_leg(
    env: &Env,
    account: &mut Account,
    action: events::PositionAction,
    hub_asset: &HubAssetKey,
    refresh_spoke: &SpokeRefresh,
    outcome: &LegOutcome,
    cache: &mut Cache,
) {
    let mut result_position = get_supply_position_or_panic(env, account, hub_asset);
    let old_scaled = result_position.scaled_amount;

    result_position.scaled_amount = outcome.new_scaled;

    cache.put_market_index(hub_asset, &outcome.market_index);
    apply_leg_usage(
        env,
        cache,
        account.spoke_id,
        UsageSide::Supply,
        hub_asset,
        LegDirection::Exit,
        old_scaled,
        outcome,
    );

    if matches!(refresh_spoke, SpokeRefresh::Refresh) && result_position.scaled_amount != Ray::ZERO
    {
        let config: AssetConfig = cache.require_spoke_asset(account.spoke_id, hub_asset);
        refresh_supply_risk_params(
            env,
            cache,
            account,
            hub_asset,
            &mut result_position,
            &config,
            RiskRefreshScope::FullTuple,
        );
    }

    update_or_remove_supply_position(account, hub_asset, &result_position);

    cache.record_supply_position_update(
        action,
        hub_asset,
        outcome.market_index.supply_index,
        outcome.amount,
        &result_position,
    );
}

/// Determines whether a withdraw leg should refresh risk parameters: returns
/// [`SpokeRefresh::Frozen`] for a liquidation, for a hub asset with no cached spoke listing, or
/// when `new_scaled` is zero; otherwise returns [`SpokeRefresh::Refresh`].
pub(crate) fn spoke_refresh_for_leg(
    kind: WithdrawKind,
    cache: &mut Cache,
    account: &Account,
    hub_asset: &HubAssetKey,
    new_scaled: Ray,
) -> SpokeRefresh {
    if kind == WithdrawKind::Liquidation {
        return SpokeRefresh::Frozen;
    }
    if cache
        .cached_spoke_asset(account.spoke_id, hub_asset)
        .is_none()
    {
        return SpokeRefresh::Frozen;
    }
    if new_scaled == Ray::ZERO {
        return SpokeRefresh::Frozen;
    }
    SpokeRefresh::Refresh
}
