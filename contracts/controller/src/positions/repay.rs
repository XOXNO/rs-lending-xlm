//! Repay and strategy-internal repay flows.
//!
//! Pipeline: auth → aggregate → cache → validate → settle → persist → emit.
//! Repay is permissionless w.r.t. the account owner and can only reduce risk;
//! the pool refunds any amount above the ceiling-rounded debt to the payer.
//! No oracle reads: repay only reduces debt and needs no live prices.

use common::errors::GenericError;
use controller_interface::types::{
    Account, DebtPosition, Payment, PoolAction, PoolPositionMutation,
};
use soroban_sdk::{contractimpl, Address, Env, Vec};

use super::{finalize_position_flow, AggregatedPayments, PositionSides};
use crate::cache::Cache;
use crate::external::pool::pool_repay_call;
use crate::helpers::update_or_remove_debt_position;
use crate::helpers::utils::{self, EventContext};
use crate::oracle::policy::OraclePolicy;
use crate::positions::{get_debt_position_or_panic, make_pool_action};
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
    pub fn repay(env: Env, caller: Address, account_id: u64, payments: Vec<(Address, i128)>) {
        process_repay(&env, &caller, account_id, &payments);
    }
}

/// Repays one or more debt assets for an account.
///
/// Account ownership is not required. The pool refunds any amount above the
/// current ceiling-rounded debt to the payer.
pub fn process_repay(env: &Env, caller: &Address, account_id: u64, payments: &Vec<Payment>) {
    caller.require_auth();
    validation::require_not_flash_loaning(env);
    validation::require_non_empty_payments(env, payments);

    let mut account = storage::get_account_borrow_only(env, account_id);
    let mut cache = Cache::new(env, OraclePolicy::Repay);

    let aggregated = utils::aggregate_positive_payments(env, payments);
    validate_repay(env, &account, &aggregated);
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

fn validate_repay(env: &Env, account: &Account, aggregated: &AggregatedPayments) {
    validation::require_non_empty_payments(env, aggregated);

    for (asset, _) in aggregated.iter() {
        let _ = get_debt_position_or_panic(env, account, &asset);
    }
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
    for (asset, amount) in aggregated.iter() {
        let position = get_debt_position_or_panic(env, account, &asset);
        let amount_in = utils::transfer_amount(
            env,
            &asset,
            caller,
            &pool_addr,
            amount,
            GenericError::AmountMustBePositive,
        );
        actions.push_back(make_pool_action(&position, amount_in, asset.clone()));
    }
    settle_repay_actions(
        env,
        account,
        caller,
        crate::events::PositionAction::Repay,
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
    action: crate::events::PositionAction,
    actions: &Vec<PoolAction>,
    cache: &mut Cache,
) -> Vec<PoolPositionMutation> {
    let pool_addr = cache.cached_pool_address();
    let results = pool_repay_call(env, &pool_addr, payer, actions);
    for (i, entry) in actions.iter().enumerate() {
        let result = validation::expect_invariant(env, results.get(i as u32));
        finish_repayment(account, action, &entry.asset, &result, cache);
    }
    results
}

/// Merges one pool repay result back into the account and event buffers.
pub(crate) fn finish_repayment(
    account: &mut Account,
    action: crate::events::PositionAction,
    asset: &Address,
    result: &PoolPositionMutation,
    cache: &mut Cache,
) {
    cache.record_market_update(&result.market_state);

    let position = DebtPosition::from(&result.position);
    update_or_remove_debt_position(account, asset, &position);

    cache.record_debt_position_update(
        action,
        asset,
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

    let mut actions: Vec<PoolAction> = Vec::new(env);
    actions.push_back(make_pool_action(
        req.position,
        req.amount,
        req.asset.clone(),
    ));
    let results = settle_repay_actions(env, account, &caller, action, &actions, cache);
    validation::expect_invariant(env, results.get(0))
}
