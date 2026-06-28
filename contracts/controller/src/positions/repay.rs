//! Repay and strategy-internal repay flows.
//! Repay is permissionless with respect to the account owner because it only
//! reduces risk. Pool refunds any amount above ceiling-rounded debt to payer.
//! No oracle reads are needed.

use common::errors::GenericError;
use common::math::fp::Ray;
use controller_interface::types::{
    Account, DebtPosition, HubAssetKey, PoolAction, PoolPositionMutation,
};
use soroban_sdk::{contractimpl, Address, Env, Vec};

use super::{finalize_position_flow, AggregatedPayments, PositionSides};
use crate::cache::Cache;
use crate::events;
use crate::external::pool::pool_repay_call;
use crate::helpers::update_or_remove_debt_position;
use crate::helpers::utils::{self, EventContext};
use crate::positions::{
    enforce_spoke_asset_flags, get_debt_position_or_panic, make_pool_action, HubPayment,
};
use crate::{storage, validation, Controller, ControllerArgs, ControllerClient};

/// Per-asset repayment inputs after the payer's transfer has been measured.
pub(crate) struct RepaymentRequest<'a> {
    pub asset: &'a Address,
    pub position: &'a DebtPosition,
    pub amount: i128,
}

#[contractimpl]
impl Controller {
    // Permissionless w.r.t. the owner: any caller authorizing itself can settle
    // another account's debt for liquidators and debt-swap strategies. Repay
    // cannot harm the owner.
    pub fn repay(env: Env, caller: Address, account_id: u64, payments: Vec<(HubAssetKey, i128)>) {
        process_repay(&env, &caller, account_id, &payments);
    }
}

/// Repays one or more debt assets for an account.
///
/// Account ownership is not required. The pool refunds any amount above the
/// current ceiling-rounded debt to the payer.
pub fn process_repay(env: &Env, caller: &Address, account_id: u64, payments: &Vec<HubPayment>) {
    caller.require_auth();
    validation::require_not_flash_loaning(env);
    validation::require_non_empty_payments(env, payments);

    let mut account = storage::get_account_borrow_only(env, account_id);
    let mut cache = Cache::new(env);

    let aggregated = utils::aggregate_positive_payments(env, payments);
    validation::require_non_empty_payments(env, &aggregated);

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
        .map(|p| Ray::from(p.scaled_amount_ray))
        .unwrap_or(Ray::ZERO);
    let position = DebtPosition::from(&result.position);
    if let Some(ctx) = cache.spoke_usage_mut(account.spoke_id) {
        // dimensional: both values are Ray<Share(asset, debt)>; repay subtracts usage.
        let delta = old_scaled - position.scaled_amount;
        ctx.apply_repay_after_pool(env, hub_asset, delta);
    }
    update_or_remove_debt_position(account, hub_asset, &position);

    cache.put_market_index(&hub_asset.asset, &result.market_index);
    cache.record_debt_position_update(
        action,
        &hub_asset.asset,
        result.market_index.borrow_index_ray,
        result.actual_amount,
        &position,
    );
}

/// Calls the pool repay path and merges the returned scaled debt share.
/// Single-asset wrapper over bulk pool repay for strategy flows.
pub fn execute_repayment(
    env: &Env,
    account: &mut Account,
    ctx: EventContext,
    req: RepaymentRequest<'_>,
    cache: &mut Cache,
) -> PoolPositionMutation {
    let EventContext { caller, action } = ctx;

    let hub_asset = HubAssetKey {
        hub_id: 0,
        asset: req.asset.clone(),
    };
    let mut actions: Vec<PoolAction> = Vec::new(env);
    actions.push_back(make_pool_action(req.position, req.amount, hub_asset));
    let results = settle_repay_actions(env, account, &caller, action, &actions, cache);
    validation::expect_invariant(env, results.get(0))
}
