use common::constants::WAD;
use common::errors::{CollateralError, GenericError};
use common::math::fp::{Ray, Wad};
use common::types::{Account, DebtPosition, Payment, PoolPositionMutation, PriceFeed};
use soroban_sdk::{contractimpl, panic_with_error, symbol_short, Address, Env, Vec};
use stellar_macros::when_not_paused;

use crate::utils::EventContext;

use crate::cache::ControllerCache;
use crate::cross_contract::pool::pool_repay_call;
use crate::helpers::{require_no_borrow_dust_for_assets, update_or_remove_debt_position};
use crate::oracle::policy::OraclePolicy;
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
    // Permissionless w.r.t. account owner: any caller can settle another
    // account's debt (required by liquidators and debt-swap strategies).
    // Repay has no side effect that could harm the owner.
    #[when_not_paused]
    pub fn repay(env: Env, caller: Address, account_id: u64, payments: Vec<Payment>) {
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
    let mut cache = ControllerCache::new(env, policy);

    // Aggregate once and reuse for the loop AND the post-flight dust scope.
    let repayment_plan = utils::aggregate_positive_payments(env, payments);
    for (asset, amount) in repayment_plan.iter() {
        process_single_repay(env, caller, &mut account, &asset, amount, &mut cache);
    }

    // Dust gate is scoped to the repaid assets — repay never mutates
    // supply positions and must not be blocked by pre-existing borrow
    // positions that drifted under the floor on assets the user did
    // not touch.
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
    cache: &mut ControllerCache,
) {
    validation::require_amount_positive(env, amount);

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
    cache: &mut ControllerCache,
) -> PoolPositionMutation {
    let EventContext { caller, action } = ctx;

    let pool_addr = cache.cached_pool_address(req.asset);
    let result = pool_repay_call(
        env,
        &pool_addr,
        caller.clone(),
        req.amount,
        req.position.into(),
    );
    cache.record_market_update_with_price(
        &result.market_state,
        if req.price > Wad::ZERO {
            Some(req.price.raw())
        } else {
            None
        },
    );

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
        if req.price > Wad::ZERO {
            Some(req.price.raw())
        } else {
            None
        },
    );

    result
}

/// Clears isolated-debt accounting for a debt position that is being removed.
pub fn clear_position_isolated_debt(
    env: &Env,
    asset: &Address,
    position: &DebtPosition,
    account: &Account,
    cache: &mut ControllerCache,
) {
    if !account.is_isolated {
        return;
    }

    let market_index = cache.cached_market_index(asset);
    let feed = cache.cached_price(asset);
    let actual_amount = actual_borrow_amount(
        env,
        position,
        market_index.borrow_index,
        feed.asset_decimals,
    );
    adjust_isolated_debt_for_repay(env, account, cache, actual_amount, &feed);
}

fn transfer_repayment_to_pool(
    env: &Env,
    caller: &Address,
    asset: &Address,
    amount: i128,
    cache: &mut ControllerCache,
) -> i128 {
    let pool_addr = cache.cached_pool_address(asset);
    utils::transfer_and_measure_received(
        env,
        asset,
        caller,
        &pool_addr,
        amount,
        GenericError::AmountMustBePositive,
    )
}

fn actual_borrow_amount(
    env: &Env,
    position: &DebtPosition,
    borrow_index: Ray,
    asset_decimals: u32,
) -> i128 {
    position
        .scaled_amount
        .mul(env, borrow_index)
        .to_asset(asset_decimals)
}

fn adjust_isolated_debt_for_repay(
    env: &Env,
    account: &Account,
    cache: &mut ControllerCache,
    actual_amount: i128,
    feed: &PriceFeed,
) {
    if account.is_isolated && actual_amount > 0 {
        adjust_isolated_debt_usd(env, account, actual_amount, feed, cache);
    }
}

fn adjust_isolated_debt_usd(
    env: &Env,
    account: &Account,
    token_amount: i128,
    feed: &PriceFeed,
    cache: &mut ControllerCache,
) {
    let Some(isolated_asset) = account.try_isolated_token() else {
        return;
    };

    let usd_wad = feed.usd_value_wad(env, token_amount).raw();

    let current = cache.get_isolated_debt(&isolated_asset);
    let mut new_debt = if usd_wad >= current {
        0
    } else {
        current - usd_wad
    };

    if new_debt > 0 && new_debt < WAD {
        new_debt = 0;
    }

    cache.set_isolated_debt(&isolated_asset, new_debt);
}
