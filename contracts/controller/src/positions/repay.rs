//! User and strategy repay: reduce debt shares; permissionless for any payer.
//!
//! Payer auth only (no owner check). Debt decreases, so no post-pool HF gate and
//! no oracle read. Not gated by `#[when_not_paused]` (spoke pause still blocks;
//! freeze does not). Liquidation skips spoke pause via bulk settle; strategy
//! `execute_repayment` requires the caller to pre-fund the pool.

use common::errors::GenericError;
use common::math::fp::Ray;
use common::types::{Account, DebtPosition, HubAssetKey, PoolAction, PoolPositionMutation};
use soroban_sdk::{contractimpl, vec, Address, Env, Vec};

use crate::account::update_or_remove_debt_position;
use crate::context::Cache;
use crate::events;
use crate::external::pool::pool_repay_call;
use crate::payments::{self, EventContext};
use crate::positions::{
    enforce_spoke_asset_flags, finalize_position_flow, get_debt_position_or_panic,
    make_pool_action, AggregatedPayments, HubPayment, PositionSides,
};
use crate::risk::validation;
use crate::storage;
use crate::{Controller, ControllerArgs, ControllerClient};

/// Single-asset repay input for strategy paths (tokens already at the pool).
pub(crate) struct RepaymentRequest<'a> {
    pub hub_asset: &'a HubAssetKey,
    pub position: &'a DebtPosition,
    pub amount: i128,
}

#[contractimpl]
impl Controller {
    /// Repays `payments` against `account_id`. Any caller may repay any account;
    /// payer auth covers the token transfer. Global pause does not block.
    ///
    /// # Errors
    /// * `FlashLoanOngoing` — a flash loan or strategy is mid-execution.
    /// * `AmountMustBePositive` — a leg amount is not strictly positive.
    /// * `SpokeAssetPaused` — spoke asset is paused (frozen may still repay).
    /// * `DebtPositionNotFound` — no debt position for an asset.
    ///
    /// # Events
    /// * topics — `["position", "batch_update"]`
    pub fn repay(env: Env, caller: Address, account_id: u64, payments: Vec<(HubAssetKey, i128)>) {
        process_repay(&env, &caller, account_id, &payments);
    }
}

/// Auth (payer), aggregate, load debt map, transfer + pool settle, persist debt.
///
/// `remove_if_empty` is false: full debt close does not remove the account here
/// (supply may still exist; withdraw owns empty-account cleanup).
pub(crate) fn process_repay(
    env: &Env,
    caller: &Address,
    account_id: u64,
    payments: &Vec<HubPayment>,
) {
    caller.require_auth();
    validation::require_not_flash_loaning(env);

    let aggregated = payments::aggregate_positive_payments(env, payments);

    let mut account = storage::get_account_borrow_only(env, account_id);
    let mut cache = Cache::new(env);

    settle_repay(env, caller, &mut account, &aggregated, &mut cache);

    finalize_position_flow(
        env,
        account_id,
        &account,
        &mut cache,
        PositionSides::DEBT,
        false,
    );
}

/// Transfer each leg into the pool, one bulk repay, merge results.
fn settle_repay(
    env: &Env,
    caller: &Address,
    account: &mut Account,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
) {
    let actions = transfer_and_build_repay_actions(env, caller, account, aggregated, cache);
    settle_repay_actions(
        env,
        account,
        caller,
        events::PositionAction::Repay,
        &actions,
        cache,
    );
}

/// Per leg: spoke pause check, require debt, transfer to pool, build pool action.
fn transfer_and_build_repay_actions(
    env: &Env,
    caller: &Address,
    account: &Account,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
) -> Vec<PoolAction> {
    let pool_addr = cache.cached_pool_address();
    let mut actions: Vec<PoolAction> = Vec::new(env);
    for (hub_asset, amount) in aggregated.iter() {
        // Paused blocks repay; frozen still allows it.
        enforce_spoke_asset_flags(env, cache, account.spoke_id, &hub_asset, false);
        let position = get_debt_position_or_panic(env, account, &hub_asset);
        let amount_in = payments::transfer_amount(
            env,
            &hub_asset.asset,
            caller,
            &pool_addr,
            amount,
            GenericError::AmountMustBePositive,
        );
        actions.push_back(make_pool_action(&position, amount_in, hub_asset.clone()));
    }
    actions
}

/// One cross-contract pool repay for `actions`, then merge results input-ordered.
///
/// Does not enforce spoke pause/freeze or transfer tokens: user path checks flags
/// and transfers first; liquidation/strategy callers are responsible for funding
/// and (for strategy) pause checks via `execute_repayment`.
pub(crate) fn settle_repay_actions(
    env: &Env,
    account: &mut Account,
    payer: &Address,
    action: events::PositionAction,
    actions: &Vec<PoolAction>,
    cache: &mut Cache,
) -> Vec<PoolPositionMutation> {
    let pool_addr = cache.cached_pool_address();
    let results = pool_repay_call(env, &pool_addr, payer, actions);
    for (i, entry) in actions.iter().enumerate() {
        let result = validation::expect_invariant(env, results.get(i as u32));
        finish_repay_leg(env, account, action, &entry.hub_asset, &result, cache);
    }
    results
}

/// Per-leg merge: debt shares, spoke usage, debt map, market index, event.
///
/// Usage delta is debt shares repaid (`old_scaled - new_scaled`). Debt position
/// is fully pool-owned.
pub(crate) fn finish_repay_leg(
    env: &Env,
    account: &mut Account,
    action: events::PositionAction,
    hub_asset: &HubAssetKey,
    result: &PoolPositionMutation,
    cache: &mut Cache,
) {
    let old_scaled = account
        .borrow_positions
        .get(hub_asset.clone())
        .map_or(Ray::ZERO, |p| Ray::from(p.scaled_amount));
    let position = DebtPosition::from(&result.position);

    let shares_repaid = old_scaled.checked_sub(env, position.scaled_amount);
    let ctx = cache.require_spoke_usage_context(account.spoke_id);
    ctx.apply_repay_after_pool(env, hub_asset, shares_repaid);

    update_or_remove_debt_position(account, hub_asset, &position);

    cache.put_market_index(hub_asset, &result.market_index);
    cache.record_debt_position_update(
        action,
        hub_asset,
        result.market_index.borrow_index,
        result.actual_amount,
        &position,
    );
}

/// Single-asset wrapper over bulk pool repay for strategy flows.
///
/// Enforces spoke pause (frozen still allowed). Does not transfer tokens — the
/// caller must already have funded the pool. Liquidation bypasses this and calls
/// `settle_repay_actions` directly.
///
/// # Security Warning
/// * Performs no `require_auth`: the calling strategy entrypoint owns
///   authorization. Repay only reduces debt.
pub(crate) fn execute_repayment(
    env: &Env,
    account: &mut Account,
    ctx: EventContext,
    req: RepaymentRequest<'_>,
    cache: &mut Cache,
) -> PoolPositionMutation {
    let EventContext {
        counterparty,
        action,
    } = ctx;

    enforce_spoke_asset_flags(env, cache, account.spoke_id, req.hub_asset, false);
    let actions = vec![
        env,
        make_pool_action(req.position, req.amount, req.hub_asset.clone()),
    ];
    let results = settle_repay_actions(env, account, &counterparty, action, &actions, cache);
    validation::expect_invariant(env, results.get(0))
}
