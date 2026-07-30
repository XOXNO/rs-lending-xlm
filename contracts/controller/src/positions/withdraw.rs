//! User and strategy withdraw: reduce supply shares and pay tokens out.
//!
//! Amount `0` maps to full-position withdraw. Debt-bearing accounts re-check
//! LTV/HF after pool indexes return. Not gated by `#[when_not_paused]` (spoke
//! pause still blocks; freeze does not). Liquidation skips spoke pause via the
//! bulk settle path.
//!
//! # Entry tiers
//!
//! Four nested entrypoints. Each tier below drops a guarantee the tier above it
//! provides, so its caller must already own that guarantee. Sections in this
//! file run in tier order, with each tier's private helpers beneath it.
//!
//! | Tier | Entrypoint | Layer it adds | Outside caller |
//! |---|---|---|---|
//! | 1 | [`process_withdraw`] | `require_auth`, owner/delegate, flash-loan guard, post-pool solvency, persist + emit | `withdraw` entrypoint |
//! | 2 | [`execute_withdrawal`] | spoke pause check, single-leg entry build | strategy legs |
//! | 3 | [`apply_withdraw_batch`] | the `pool.withdraw` call over pre-built entries | liquidation seize |
//! | 4 | [`merge_withdraw_leg`] | base: folds one pool result into account, spoke usage, index memo, event buffer | net settle |

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

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// How a withdraw batch executes. Selects the pool's liquidation semantics and
/// whether collateral risk params stay at their stamped vintage.
///
/// Deliberately independent of [`events::PositionAction`]: the event taxonomy is
/// an ABI concern and must not decide money-path behavior.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum WithdrawKind {
    /// User or strategy withdraw. The pool takes its normal path and every
    /// still-listed leg is restamped from live spoke config.
    Normal,
    /// Liquidation seizure. The pool takes its liquidation path and risk params
    /// stay frozen, so a delisted or frozen leg is still seizable.
    Liquidation,
}

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

// ---------------------------------------------------------------------------
// Tier 1: user withdraw
// ---------------------------------------------------------------------------

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
    require_position_caller(env, caller);

    let mut account = storage::get_account(env, account_id);
    require_owner_or_delegate(env, account_id, caller, &account.owner);

    let recipient = to.unwrap_or_else(|| caller.clone());
    let mut cache = Cache::new(env);
    // `zero_is_withdraw_all: true` keeps amount `0` as a full-withdraw sentinel.
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

/// Build entries, one bulk pool withdraw, return paid amounts in input order.
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

    // `apply_withdraw_batch` already asserted one result per entry.
    let mut paid: Vec<HubPayment> = Vec::new(env);
    for (entry, result) in entries.iter().zip(results.iter()) {
        paid.push_back((entry.action.hub_asset, result.actual_amount));
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

/// Maps a caller's `0` to the full-position sentinel. Bulk path only —
/// [`execute_withdrawal`] forwards its amount untouched, so strategy callers pass
/// [`WITHDRAW_ALL_SENTINEL`] themselves.
fn resolve_withdraw_amount(amount: i128) -> i128 {
    if amount == 0 {
        WITHDRAW_ALL_SENTINEL
    } else {
        amount
    }
}

// ---------------------------------------------------------------------------
// Tier 2: single-asset strategy withdraw
// ---------------------------------------------------------------------------

/// Single-asset wrapper over bulk pool withdraw for strategy and account-close.
///
/// Enforces spoke pause (frozen still allowed). Liquidation bypasses this and
/// calls `apply_withdraw_batch` directly.
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

// ---------------------------------------------------------------------------
// Tier 3: bulk pool withdraw
// ---------------------------------------------------------------------------

/// One cross-contract pool withdraw for `entries`, then merge results input-ordered.
///
/// Does not enforce spoke pause/freeze: user and strategy paths check flags
/// before calling; liquidation calls this directly and stays exempt.
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

// ---------------------------------------------------------------------------
// Tier 4: single-leg account merge
// ---------------------------------------------------------------------------

/// Per-leg merge: scaled shares, usage, optional risk refresh, supply map, event.
///
/// Risk params refresh only when `refresh_spoke` is `Refresh` and the leg leaves
/// shares behind.
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
    // Pool owns scaled shares; controller keeps collateral risk params unless refreshing.
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

    // Backstop, not a duplicate: `refresh_spoke` and `outcome` arrive independently
    // and callers reach this leaf directly, so do not trust that they agree.
    if matches!(refresh_spoke, SpokeRefresh::Refresh) && result_position.scaled_amount != Ray::ZERO
    {
        let config: AssetConfig = (&cache.require_spoke_asset(account.spoke_id, hub_asset)).into();
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

// ---------------------------------------------------------------------------
// Risk-param refresh policy (shared by tiers 3 and 4)
// ---------------------------------------------------------------------------

/// Whether a withdraw leg restamps risk params from live spoke config, or keeps
/// the vintage stamped at supply time. Any one reason freezes it:
/// * liquidation — a seizure settles against the vintage it was priced on, so a
///   delisted or frozen leg stays seizable;
/// * the asset is no longer listed, so there is no live config;
/// * the leg emptied the position ([`merge_withdraw_leg`] rechecks this itself).
///
/// Order is load-bearing: `cached_spoke_asset` renews the listing TTL on a memo
/// miss, so the emptied check must not short-circuit past it.
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
