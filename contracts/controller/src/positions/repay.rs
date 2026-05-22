use common::errors::{CollateralError, GenericError};
use common::math::fp::{Ray, Wad};
use common::types::{
    Account, AccountPosition, AccountPositionType, Payment, PoolPositionMutation, PriceFeed,
};
use soroban_sdk::{contractimpl, panic_with_error, symbol_short, Address, Env, Map, Vec};
use stellar_macros::when_not_paused;

use super::EventContext;

use super::dust::require_no_dust_after;
use super::update;
use crate::cache::ControllerCache;
use crate::cross_contract::pool::pool_repay_call;
use crate::oracle::policy::OraclePolicy;
use crate::{storage, utils, validation, Controller, ControllerArgs, ControllerClient};

/// Bundle of per-call repayment inputs.
pub(crate) struct RepaymentRequest<'a> {
    pub asset: &'a Address,
    pub position: &'a AccountPosition,
    pub amount: i128,
    pub price: Wad,
}

#[contractimpl]
impl Controller {
    #[when_not_paused]
    pub fn repay(env: Env, caller: Address, account_id: u64, payments: Vec<(Address, i128)>) {
        process_repay(&env, &caller, account_id, &payments);
    }
}

// Processes repay batch.
pub fn process_repay(env: &Env, caller: &Address, account_id: u64, payments: &Vec<Payment>) {
    caller.require_auth();
    validation::require_not_flash_loaning(env);
    validation::require_non_empty_payments(env, payments);

    let meta = storage::get_account_meta(env, account_id);
    let borrow_positions = storage::get_positions(env, account_id, AccountPositionType::Borrow);
    // Isolated accounts use safe prices for counter decrements.
    let mut account = storage::account_from_parts(meta, Map::new(env), borrow_positions);
    let policy = if account.is_isolated {
        OraclePolicy::IsolatedRepay
    } else {
        OraclePolicy::Repay
    };
    let mut cache = ControllerCache::new(env, policy);

    let repayment_plan = utils::aggregate_positive_payments(env, payments);
    for (asset, amount) in repayment_plan {
        process_single_repay(env, caller, &mut account, &asset, amount, &mut cache);
    }

    // Partial repay must stay above dust floor or repay in full.
    require_no_dust_after(env, &mut cache, &account);

    // Repay does not delete account storage.
    storage::set_positions(
        env,
        account_id,
        AccountPositionType::Borrow,
        &account.borrow_positions,
    );
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

    let position: AccountPosition = (&account
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
            event_caller: caller.clone(),
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

// Executes pool repay.
pub fn execute_repayment(
    env: &Env,
    account: &mut Account,
    ctx: EventContext,
    req: RepaymentRequest<'_>,
    cache: &mut ControllerCache,
) -> PoolPositionMutation {
    let EventContext {
        caller,
        event_caller,
        action,
    } = ctx;

    let pool_addr = cache.cached_pool_address(req.asset);
    let result = pool_repay_call(env, &pool_addr, caller.clone(), req.amount, req.position.clone());
    cache.record_market_update_with_price(
        &result.market_state,
        if req.price > Wad::ZERO {
            Some(req.price.raw())
        } else {
            None
        },
    );

    update::update_or_remove_position(
        account,
        AccountPositionType::Borrow,
        req.asset,
        &AccountPosition::from(&result.position),
    );
    if account.is_isolated {
        let feed = cache.cached_price(req.asset);
        adjust_isolated_debt_for_repay(env, account, cache, result.actual_amount, &feed);
    }
    let _ = event_caller;
    cache.record_position_update(
        action,
        AccountPositionType::Borrow,
        req.asset,
        result.market_index.borrow_index_ray,
        result.actual_amount,
        &AccountPosition::from(&result.position),
        if req.price > Wad::ZERO {
            Some(req.price.raw())
        } else {
            None
        },
    );

    result
}

// Clears isolated debt.
pub fn clear_position_isolated_debt(
    env: &Env,
    asset: &Address,
    position: &AccountPosition,
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
    position: &AccountPosition,
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
        utils::adjust_isolated_debt_usd(env, account, actual_amount, feed, cache);
    }
}
