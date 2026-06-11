use common::errors::{CollateralError, GenericError};
use common::types::{Account, DebtPosition, Payment, PoolAction, PoolPositionMutation};
use soroban_sdk::{contractimpl, panic_with_error, Address, Env, Vec};
use stellar_macros::when_not_paused;

use crate::utils::EventContext;

use crate::cache::Cache;
use crate::cross_contract::pool::pool_repay_call;
use crate::helpers::{require_no_borrow_dust_for_assets, update_or_remove_debt_position};
use crate::oracle::policy::OraclePolicy;
use crate::positions::isolated_debt::adjust_isolated_debt_for_repay;
use crate::{storage, utils, validation, Controller, ControllerArgs, ControllerClient};

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
    // Build the whole plan's actions (measuring each transfer first), make
    // ONE pool call, then merge results input-ordered — one cross-contract
    // frame instead of one per asset.
    let mut actions: Vec<PoolAction> = Vec::new(env);
    for (asset, amount) in repayment_plan.iter() {
        validation::require_positive_amount(env, amount);
        let position: DebtPosition = (&account
            .borrow_positions
            .get(asset.clone())
            .unwrap_or_else(|| panic_with_error!(env, CollateralError::PositionNotFound)))
            .into();
        let actual_received = transfer_repayment_to_pool(env, caller, &asset, amount, &mut cache);
        actions.push_back(PoolAction {
            position: (&position).into(),
            amount: actual_received,
            asset: asset.clone(),
        });
    }
    let pool_addr = cache.cached_pool_address();
    let results = pool_repay_call(env, &pool_addr, caller, &actions);
    for (i, action) in actions.iter().enumerate() {
        let result = validation::expect_invariant(env, results.get(i as u32));
        finish_repayment(
            env,
            &mut account,
            common::events::PositionAction::Repay,
            &action.asset,
            &result,
            &mut cache,
        );
    }

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

/// Calls the pool repay path and merges the returned scaled debt share.
/// Single-asset wrapper over the bulk pool repay — used by strategies and the
/// liquidation repay leg's bulk-of-one callers (same frame cost as the old
/// single endpoint).
pub fn execute_repayment(
    env: &Env,
    account: &mut Account,
    ctx: EventContext,
    req: RepaymentRequest<'_>,
    cache: &mut Cache,
) -> PoolPositionMutation {
    let EventContext { caller, action } = ctx;

    let pool_addr = cache.cached_pool_address();
    let mut actions: Vec<PoolAction> = Vec::new(env);
    actions.push_back(PoolAction {
        position: req.position.into(),
        amount: req.amount,
        asset: req.asset.clone(),
    });
    let results = pool_repay_call(env, &pool_addr, &caller, &actions);
    let result = validation::expect_invariant(env, results.get(0));
    finish_repayment(env, account, action, req.asset, &result, cache);
    result
}

/// Merges one pool repay result back into the account and event buffers.
pub(crate) fn finish_repayment(
    env: &Env,
    account: &mut Account,
    action: common::events::PositionAction,
    asset: &Address,
    result: &PoolPositionMutation,
    cache: &mut Cache,
) {
    cache.record_market_update(&result.market_state);

    update_or_remove_debt_position(account, asset, &DebtPosition::from(&result.position));

    if account.is_isolated {
        let feed = cache.cached_price(asset);
        adjust_isolated_debt_for_repay(env, account, cache, result.actual_amount, &feed);
    }

    cache.record_debt_position_update(
        action,
        asset,
        result.market_index.borrow_index_ray,
        result.actual_amount,
        &DebtPosition::from(&result.position),
    );
}

fn transfer_repayment_to_pool(
    env: &Env,
    caller: &Address,
    asset: &Address,
    amount: i128,
    cache: &mut Cache,
) -> i128 {
    let pool_addr = cache.cached_pool_address();
    utils::transfer_amount(
        env,
        asset,
        caller,
        &pool_addr,
        amount,
        GenericError::AmountMustBePositive,
    )
}
