use common::errors::{CollateralError, GenericError};
use common::math::fp::Wad;
use common::types::{Account, DebtPosition, Payment, PoolAction, PoolPositionMutation};
use soroban_sdk::{contractimpl, panic_with_error, symbol_short, Address, Env, Vec};
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
    pub price: Wad,
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
    // The isolated path prices each repaid asset in `process_single_repay`
    // before the dust gate's own prefetch runs.  Bulk-fetch the plan assets
    // here so all per-asset `cached_price` calls in the loop hit the cache
    // rather than single-resolving separate RedStone feeds.  The dust gate's
    // prefetch then no-ops because the collector is idempotent.
    crate::oracle::prefetch_redstone_feeds(&mut cache, &utils::plan_assets(env, &repayment_plan));
    for (asset, amount) in repayment_plan.iter() {
        process_single_repay(env, caller, &mut account, &asset, amount, &mut cache);
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

fn process_single_repay(
    env: &Env,
    caller: &Address,
    account: &mut Account,
    asset: &Address,
    amount: i128,
    cache: &mut Cache,
) {
    validation::require_positive_amount(env, amount);

    let position: DebtPosition = (&account
        .borrow_positions
        .get(asset.clone())
        .unwrap_or_else(|| panic_with_error!(env, CollateralError::PositionNotFound)))
        .into();
    let actual_received = transfer_repayment_to_pool(env, caller, asset, amount, cache);

    let price = if account.is_isolated {
        cache.cached_price(asset).price
    } else {
        Wad::ZERO
    };
    let _ = execute_repayment(
        env,
        account,
        EventContext {
            caller: caller.clone(),
            action: symbol_short!("repay"),
        },
        RepaymentRequest {
            asset,
            position: &position,
            amount: actual_received,
            price,
        },
        cache,
    );
}

/// Calls the pool repay path and merges the returned scaled debt share.
pub fn execute_repayment(
    env: &Env,
    account: &mut Account,
    ctx: EventContext,
    req: RepaymentRequest<'_>,
    cache: &mut Cache,
) -> PoolPositionMutation {
    let EventContext { caller, action } = ctx;

    let price_opt = if req.price > Wad::ZERO {
        Some(req.price.raw())
    } else {
        None
    };

    let pool_addr = cache.cached_pool_address();
    let pool_action = PoolAction {
        caller: caller.clone(),
        position: req.position.into(),
        amount: req.amount,
        asset: req.asset.clone(),
    };
    let result = pool_repay_call(env, &pool_addr, pool_action);
    cache.record_market_update_with_price(&result.market_state, price_opt);

    update_or_remove_debt_position(account, req.asset, &DebtPosition::from(&result.position));

    if account.is_isolated {
        let feed = cache.cached_price(req.asset);
        adjust_isolated_debt_for_repay(env, account, cache, result.actual_amount, &feed);
    }

    cache.record_debt_position_update(
        action,
        req.asset,
        result.market_index.borrow_index_ray,
        result.actual_amount,
        &DebtPosition::from(&result.position),
        price_opt,
    );

    result
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
