//! Repay and strategy-internal repay flows.
//!
//! Repay is permissionless w.r.t. the account owner and can only reduce risk;
//! the pool refunds any amount above the ceiling-rounded debt to the payer.

use common::errors::GenericError;
use common::types::{Account, DebtPosition, Payment, PoolAction, PoolPositionMutation};
use soroban_sdk::{contractimpl, Address, Env, Vec};
use stellar_macros::when_not_paused;

use crate::cache::Cache;
use crate::external::pool::pool_repay_call;
use crate::helpers::utils::{self, EventContext};
use crate::helpers::{require_no_borrow_dust_for_assets, update_or_remove_debt_position};
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
    #[when_not_paused]
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

    // Aggregate once and reuse for the loop AND the post-flight dust scope.
    let repayment_plan = utils::aggregate_positive_payments(env, payments);
    // Non-isolated repay sets price = Wad::ZERO for every asset, so the loop
    // prices nothing; any pricing the dust gate still needs (partially repaid
    // positions) is covered by the gate's own prefetch. An entrypoint
    // prefetch here would add calls for nothing — skip it.
    if account.is_isolated {
        // Prefetch only assets the account actually owes; unknown or
        // position-less assets are rejected by the loop below with their
        // pre-existing error codes.
        let mut owed: Vec<Address> = Vec::new(env);
        for (asset, _) in repayment_plan.iter() {
            if account.borrow_positions.contains_key(asset.clone()) {
                owed.push_back(asset);
            }
        }
        crate::oracle::prefetch_redstone_feeds(&mut cache, &owed);
    }
    // Transfer each repayment in while building the plan's actions for one
    // bulk pool call.
    let pool_addr = cache.cached_pool_address();
    let mut actions: Vec<PoolAction> = Vec::new(env);
    for (asset, amount) in repayment_plan.iter() {
        let position = get_debt_position_or_panic(env, &account, &asset);
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
        &mut account,
        account_id,
        caller,
        common::events::PositionAction::Repay,
        &actions,
        &mut cache,
    );

    // Dust gate scoped to repaid assets: repay never mutates supply positions,
    // so untouched borrows that drifted under floor must not block the call.
    require_no_borrow_dust_for_assets(
        env,
        &mut cache,
        &account,
        &utils::plan_assets(env, &repayment_plan),
    );

    storage::set_debt_positions(env, account_id, &account.borrow_positions);
    cache.flush_isolated_debts();
    cache.emit_position_batch(account_id, &account);
    cache.emit_market_batch();
}

/// Executes one bulk pool repay for `actions` (one cross-contract frame) and
/// merges the results input-ordered.
pub(crate) fn settle_repay_actions(
    env: &Env,
    account: &mut Account,
    account_id: u64,
    payer: &Address,
    action: common::events::PositionAction,
    actions: &Vec<PoolAction>,
    cache: &mut Cache,
) -> Vec<PoolPositionMutation> {
    let pool_addr = cache.cached_pool_address();
    let results = pool_repay_call(env, &pool_addr, payer, actions);
    for (i, entry) in actions.iter().enumerate() {
        let result = validation::expect_invariant(env, results.get(i as u32));
        finish_repayment(env, account, account_id, action, &entry.asset, &result, cache);
    }
    results
}

/// Merges one pool repay result back into the account and event buffers.
pub(crate) fn finish_repayment(
    env: &Env,
    account: &mut Account,
    account_id: u64,
    action: common::events::PositionAction,
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
