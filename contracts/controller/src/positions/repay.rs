//! Repay flows. Permissionless; only reduces debt and uses no oracle reads.

use common::errors::GenericError;
use common::math::fp::Ray;
use common::types::{Account, DebtPosition, HubAssetKey, PoolAction, PoolPositionMutation};
use soroban_sdk::{contractimpl, Address, Env, Vec};

use crate::account::update_or_remove_debt_position;
use crate::context::Cache;
use crate::events;
use crate::external::pool::pool_repay_call;
use crate::payments::{self as utils, EventContext};
use crate::positions::{
    enforce_spoke_asset_flags, get_debt_position_or_panic, make_pool_action, HubPayment,
};
use crate::positions::{finalize_position_flow, AggregatedPayments, PositionSides};
use crate::{risk::validation, storage, Controller, ControllerArgs, ControllerClient};

/// Per-asset repayment input.
pub(crate) struct RepaymentRequest<'a> {
    pub hub_asset: &'a HubAssetKey,
    pub position: &'a DebtPosition,
    pub amount: i128,
}

#[contractimpl]
impl Controller {
    /// Repays one or more debt assets. Permissionless: any caller may repay any
    /// account's debt (it only reduces debt, so no owner auth is required).
    ///
    /// # Arguments
    /// * `caller` - the payer; must authorize the token transfer.
    /// * `payments` - `(hub-asset, amount)` repayment legs; amounts must be positive.
    ///
    /// # Errors
    /// * `FlashLoanOngoing` - a flash loan or strategy is mid-execution.
    /// * `AmountMustBePositive` - a leg amount is not strictly positive.
    /// * `SpokeAssetPaused` - the spoke asset is paused (a frozen asset may still be repaid).
    /// * `DebtPositionNotFound` - the account holds no debt position for an asset.
    ///
    /// # Events
    /// * A position-batch event summarizing the account's reduced debt legs.
    pub fn repay(env: Env, caller: Address, account_id: u64, payments: Vec<(HubAssetKey, i128)>) {
        process_repay(&env, &caller, account_id, &payments);
    }
}

/// Repays one or more debt assets.
pub fn process_repay(env: &Env, caller: &Address, account_id: u64, payments: &Vec<HubPayment>) {
    caller.require_auth();
    validation::require_not_flash_loaning(env);

    // Input validation (non-empty, positive amounts) precedes the account read.
    let aggregated = utils::aggregate_positive_payments(env, payments);

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

fn settle_repay(
    env: &Env,
    caller: &Address,
    account: &mut Account,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
) {
    let pool_addr = cache.cached_pool_address();
    let mut actions: Vec<PoolAction> = Vec::new(env);
    for (hub_asset, amount) in aggregated.iter() {
        // Paused blocks repay; frozen still allows it.
        enforce_spoke_asset_flags(env, cache, account.spoke_id, &hub_asset, false);
        let position = get_debt_position_or_panic(env, account, &hub_asset);
        let amount_in = utils::transfer_amount(
            env,
            &hub_asset.asset,
            caller,
            &pool_addr,
            amount,
            GenericError::AmountMustBePositive,
        );
        actions.push_back(make_pool_action(&position, amount_in, hub_asset.clone()));
    }
    settle_repay_actions(
        env,
        account,
        caller,
        events::PositionAction::Repay,
        &actions,
        cache,
    );
}

/// Executes one bulk pool repay for `actions` (one cross-contract frame) and
/// merges the results input-ordered.
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
        finish_repayment(env, account, action, &entry.hub_asset, &result, cache);
    }
    results
}

/// Merges one pool repay result back into the account and event buffers.
pub(crate) fn finish_repayment(
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
        .map(|p| Ray::from(p.scaled_amount))
        .unwrap_or(Ray::ZERO);
    let position = DebtPosition::from(&result.position);
    let ctx = cache.require_spoke_usage_context(account.spoke_id);
    // dimensional: both values are Ray<Share(asset, debt)>; repay subtracts usage.
    let delta = old_scaled - position.scaled_amount;
    ctx.apply_repay_after_pool(env, hub_asset, delta);
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

/// Single-asset wrapper over the bulk pool repay for strategy flows. Enforces the
/// per-spoke paused flag (frozen still allows repay); liquidation bypasses this
/// via `settle_repay_actions`.
///
/// # Security Warning
/// * Performs no `require_auth`: the calling strategy entrypoint owns
///   authorization. Repay only reduces debt.
pub fn execute_repayment(
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

    // Strategy chokepoint: paused blocks repay, frozen still allows it.
    // Liquidation calls `settle_repay_actions` directly and stays exempt.
    enforce_spoke_asset_flags(env, cache, account.spoke_id, req.hub_asset, false);
    let mut actions: Vec<PoolAction> = Vec::new(env);
    actions.push_back(make_pool_action(
        req.position,
        req.amount,
        req.hub_asset.clone(),
    ));
    let results = settle_repay_actions(env, account, &counterparty, action, &actions, cache);
    validation::expect_invariant(env, results.get(0))
}
