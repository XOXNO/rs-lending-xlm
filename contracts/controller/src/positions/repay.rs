//! Repay and strategy-internal repay flows.
//!
//! Pipeline: auth → aggregate → cache → validate → settle → persist → emit.
//! Repay is permissionless w.r.t. the account owner and can only reduce risk;
//! the pool refunds any amount above the ceiling-rounded debt to the payer.

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
use crate::positions::isolated_debt::adjust_isolated_debt_for_repay;
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
    // Permissionless w.r.t. the owner: any caller (authorizing itself) can
    // settle another account's debt — needed by liquidators and debt-swap
    // strategies — and repay can't harm the owner.
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
    let policy = if account.is_isolated {
        OraclePolicy::IsolatedRepay
    } else {
        OraclePolicy::Repay
    };
    let mut cache = Cache::new(env, policy);

    let aggregated = utils::aggregate_positive_payments(env, payments);
    validate_repay(env, &account, &aggregated, &mut cache);
    settle_repay(
        env,
        caller,
        &mut account,
        account_id,
        &aggregated,
        &mut cache,
    );

    finalize_position_flow(
        env,
        account_id,
        &account,
        &mut cache,
        PositionSides::DEBT,
        false,
        true,
    );
}

fn validate_repay(
    env: &Env,
    account: &Account,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
) {
    validation::require_non_empty_payments(env, aggregated);
    prefetch_isolated_repay_oracles(env, cache, account, aggregated);

    for (asset, _) in aggregated.iter() {
        let _ = get_debt_position_or_panic(env, account, &asset);
    }
}

fn settle_repay(
    env: &Env,
    caller: &Address,
    account: &mut Account,
    account_id: u64,
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
        account_id,
        caller,
        crate::events::PositionAction::Repay,
        &actions,
        cache,
    );
}

/// Isolated repay prices each repaid asset for the debt-ceiling adjustment.
/// Non-isolated repay uses zero prices, so prefetch is skipped entirely.
fn prefetch_isolated_repay_oracles(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    aggregated: &AggregatedPayments,
) {
    if !account.is_isolated {
        return;
    }
    let mut owed: Vec<Address> = Vec::new(env);
    for (asset, _) in aggregated.iter() {
        owed.push_back(asset);
    }
    crate::oracle::prefetch_redstone_feeds(cache, &owed);
}

/// Executes one bulk pool repay for `actions` (one cross-contract frame) and
/// merges the results input-ordered.
pub(crate) fn settle_repay_actions(
    env: &Env,
    account: &mut Account,
    account_id: u64,
    payer: &Address,
    action: crate::events::PositionAction,
    actions: &Vec<PoolAction>,
    cache: &mut Cache,
) -> Vec<PoolPositionMutation> {
    let pool_addr = cache.cached_pool_address();
    let results = pool_repay_call(env, &pool_addr, payer, actions);
    for (i, entry) in actions.iter().enumerate() {
        let result = validation::expect_invariant(env, results.get(i as u32));
        finish_repayment(
            env,
            account,
            account_id,
            action,
            &entry.asset,
            &result,
            cache,
        );
    }
    results
}

/// Merges one pool repay result back into the account and event buffers.
pub(crate) fn finish_repayment(
    env: &Env,
    account: &mut Account,
    account_id: u64,
    action: crate::events::PositionAction,
    asset: &Address,
    result: &PoolPositionMutation,
    cache: &mut Cache,
) {
    cache.record_market_update(&result.market_state);

    // Capture the pre-repay scaled debt before the position is overwritten;
    // the isolated-debt decrement is proportional to the share repaid.
    let scaled_before = account
        .borrow_positions
        .get(asset.clone())
        .map(|raw| common::math::fp::Ray::from(raw.scaled_amount_ray))
        .unwrap_or(common::math::fp::Ray::ZERO);

    let position = DebtPosition::from(&result.position);
    update_or_remove_debt_position(account, asset, &position);

    if account.is_isolated {
        adjust_isolated_debt_for_repay(
            env,
            account,
            account_id,
            cache,
            asset,
            scaled_before,
            position.scaled_amount,
        );
    }

    cache.record_debt_position_update(
        action,
        asset,
        result.market_index.borrow_index_ray,
        result.actual_amount,
        &position,
    );
}

/// Calls the pool repay path and merges the returned scaled debt share.
/// Single-asset wrapper over the bulk pool repay — used by strategy flows.
pub fn execute_repayment(
    env: &Env,
    account: &mut Account,
    account_id: u64,
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
    let results = settle_repay_actions(env, account, account_id, &caller, action, &actions, cache);
    validation::expect_invariant(env, results.get(0))
}
