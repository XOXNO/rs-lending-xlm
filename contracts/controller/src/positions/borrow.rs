//! User and strategy borrow: pool pays proceeds out; no pre-transfer.
//!
//! User path re-checks LTV/HF after pool indexes return. Strategy helpers share
//! entry gates and merge logic but defer post-pool risk gates to
//! `strategy_finalize`. See `architecture/INVARIANTS.md` §3.2.

use common::math::fp::Ray;
use common::types::{
    Account, AccountPositionType, DebtPosition, HubAssetKey, PoolBorrowEntry, PoolPositionMutation,
};
use soroban_sdk::{contractimpl, vec, Address, Env, Vec};
use stellar_macros::when_not_paused;

use crate::account::{require_owner_or_delegate, update_or_remove_debt_position};
use crate::context::Cache;
use crate::events;
use crate::external::pool::{pool_borrow_call, pool_create_strategy_call};
use crate::payments;
use crate::positions::{
    finalize_position_flow, make_pool_action, validate_position_entry_gates, AggregatedPayments,
    HubPayment, PositionSides,
};
use crate::risk::{self, validation};
use crate::storage;
use crate::{Controller, ControllerArgs, ControllerClient};

#[contractimpl]
impl Controller {
    /// Borrows `borrows` to `to` (default `caller`) on an existing account.
    /// Owner or active delegate. Re-checks LTV/HF on pool-returned indexes.
    ///
    /// # Errors
    /// * `NotAuthorized` — `caller` is neither owner nor active delegate.
    /// * `FlashLoanOngoing` — a flash loan or strategy is mid-execution.
    /// * `HubNotActive` / `AssetNotInSpoke` / `SpokeAssetPaused` / `SpokeAssetFrozen` /
    ///   `AssetNotBorrowable` / `PositionLimitExceeded` — entry gates.
    /// * `SpokeBorrowCapReached` — borrow would exceed the spoke borrow cap.
    /// * `BorrowRoundsToZeroShares` — amount rounds to zero scaled debt (pool).
    /// * `InsufficientCollateral` / `MinBorrowCollateralNotMet` — post-pool risk gates.
    /// * The `#[when_not_paused]` guard reverts while the contract is paused.
    ///
    /// # Events
    /// * topics — `["position", "batch_update"]`
    #[when_not_paused]
    pub fn borrow(
        env: Env,
        caller: Address,
        account_id: u64,
        borrows: Vec<(HubAssetKey, i128)>,
        to: Option<Address>,
    ) {
        process_borrow(&env, &caller, account_id, &borrows, to);
    }
}

/// Auth, load account, entry gates, pool borrow, post-pool solvency, then persist.
///
/// `remove_if_empty` is false: this path only increases debt, so account cleanup
/// is never needed here.
pub(crate) fn process_borrow(
    env: &Env,
    caller: &Address,
    account_id: u64,
    borrows: &Vec<HubPayment>,
    to: Option<Address>,
) {
    caller.require_auth();
    validation::require_not_flash_loaning(env);

    let mut account = storage::get_account(env, account_id);
    require_owner_or_delegate(env, account_id, caller, &account.owner);

    let recipient = to.unwrap_or_else(|| caller.clone());
    let mut cache = Cache::new(env);
    let aggregated = payments::aggregate_positive_payments(env, borrows);

    validate_position_entry_gates(
        env,
        &account,
        &aggregated,
        &mut cache,
        AccountPositionType::Borrow,
    );
    settle_borrow(env, &recipient, &mut account, &aggregated, &mut cache);

    let restamped = risk::restamp_listed_supply_safe_params(&mut cache, &mut account);
    validation::require_post_pool_risk_gates(env, &mut cache, &account);

    let sides = if restamped {
        PositionSides::BOTH
    } else {
        PositionSides::DEBT
    };
    finalize_position_flow(env, account_id, &account, &mut cache, sides, false);
}

/// One batch `pool.borrow` to `recipient`, then merge input-ordered results.
fn settle_borrow(
    env: &Env,
    recipient: &Address,
    account: &mut Account,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
) {
    let entries = build_borrow_entries(env, account, aggregated);
    let pool_addr = cache.cached_pool_address();
    let results = pool_borrow_call(env, &pool_addr, recipient, &entries);
    apply_borrow_results(env, account, &entries, &results, cache);
}

/// Snapshots each debt leg for the pool action (in-memory; merge persists).
fn build_borrow_entries(
    env: &Env,
    account: &Account,
    aggregated: &AggregatedPayments,
) -> Vec<PoolBorrowEntry> {
    let mut entries: Vec<PoolBorrowEntry> = Vec::new(env);
    for (hub_asset, amount) in aggregated {
        let borrow_position = account.get_or_create_debt_position(&hub_asset);
        entries.push_back(PoolBorrowEntry {
            action: make_pool_action(&borrow_position, amount, hub_asset.clone()),
        });
    }
    entries
}

/// Input-ordered pool results → `finish_borrow_leg` per entry.
fn apply_borrow_results(
    env: &Env,
    account: &mut Account,
    entries: &Vec<PoolBorrowEntry>,
    results: &Vec<PoolPositionMutation>,
    cache: &mut Cache,
) {
    for (i, entry) in entries.iter().enumerate() {
        let result = validation::expect_invariant(env, results.get(i as u32));
        finish_borrow_leg(
            env,
            account,
            &entry.action.hub_asset,
            events::PositionAction::Borrow,
            &result,
            cache,
        );
    }
}

/// Per-leg merge: debt position, spoke usage, market index, event, debt map.
fn finish_borrow_leg(
    env: &Env,
    account: &mut Account,
    hub_asset: &HubAssetKey,
    action: events::PositionAction,
    result: &PoolPositionMutation,
    cache: &mut Cache,
) {
    let old_scaled = account
        .borrow_positions
        .get(hub_asset.clone())
        .map_or(Ray::ZERO, |p| Ray::from(p.scaled_amount));
    // Debt position is fully pool-owned (scaled shares); no controller risk params.
    let position: DebtPosition = DebtPosition::from(&result.position);

    let delta = position.scaled_amount.checked_sub(env, old_scaled);
    let ctx = cache.require_spoke_usage_context(account.spoke_id);
    ctx.apply_borrow_after_pool(
        env,
        hub_asset,
        delta,
        &result.market_index,
        result.asset_decimals,
    );

    cache.put_market_index(hub_asset, &result.market_index);
    cache.record_debt_position_update(
        action,
        hub_asset,
        result.market_index.borrow_index,
        result.actual_amount,
        &position,
    );
    update_or_remove_debt_position(account, hub_asset, &position);
}

/// and returns the asset amount received by the controller.
///
/// Used by multiply and swap-debt. Charges the market's configured flash-loan fee.
///
/// # Security Warning
/// * Performs no `require_auth`: authorization is enforced by the strategy
///   entrypoint that invokes it. Post-borrow solvency is deferred to
///   `strategy_finalize`. Never call from an un-authorized context.
pub(crate) fn borrow_for_strategy(
    env: &Env,
    account: &mut Account,
    hub_debt: &HubAssetKey,
    amount: i128,
    cache: &mut Cache,
) -> i128 {
    borrow_strategy_inner(
        env,
        account,
        hub_debt,
        amount,
        cache,
        true,
        events::PositionAction::Multiply,
    )
}

/// Zero-fee strategy borrow for Blend migration. Proceeds go to the controller.
///
/// # Security Warning
/// * Performs no `require_auth`: authorization is enforced by the migration
///   entrypoint that invokes it. Solvency is deferred to `strategy_finalize`.
pub(crate) fn borrow_for_migration(
    env: &Env,
    account: &mut Account,
    hub_debt: &HubAssetKey,
    amount: i128,
    cache: &mut Cache,
) -> i128 {
    borrow_strategy_inner(
        env,
        account,
        hub_debt,
        amount,
        cache,
        false,
        events::PositionAction::Migrate,
    )
}

/// Shared strategy-borrow body.
///
/// `charge_fee`: `true` applies the market flash-loan fee (multiply); `false`
/// borrows fee-free (migration). The fee amount is computed pool-side from the
/// market's `flashloan_fee` bps. Entry gates run; post-pool HF does not.
fn borrow_strategy_inner(
    env: &Env,
    account: &mut Account,
    hub_debt: &HubAssetKey,
    amount: i128,
    cache: &mut Cache,
    charge_fee: bool,
    event_action: events::PositionAction,
) -> i128 {
    let hub_debt = hub_debt.clone();
    let payments: AggregatedPayments = vec![env, (hub_debt.clone(), amount)];
    let aggregated = payments::aggregate_positive_payments(env, &payments);
    validate_position_entry_gates(
        env,
        account,
        &aggregated,
        cache,
        AccountPositionType::Borrow,
    );

    let borrow_position = account.get_or_create_debt_position(&hub_debt);

    let pool_addr = cache.cached_pool_address();
    let pool_action = make_pool_action(&borrow_position, amount, hub_debt.clone());
    let result = pool_create_strategy_call(
        env,
        &pool_addr,
        &env.current_contract_address(),
        pool_action,
        charge_fee,
    );
    let mutation: PoolPositionMutation = PoolPositionMutation::from(&result);
    finish_borrow_leg(env, account, &hub_debt, event_action, &mutation, cache);

    result.amount_received
}
