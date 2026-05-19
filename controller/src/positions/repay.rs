use common::errors::{CollateralError, GenericError};
use common::fp::Ray;
use common::types::{Account, AccountPosition, AccountPositionType, Payment, PoolPositionMutation};
use soroban_sdk::{contractimpl, panic_with_error, symbol_short, Address, Env, Map, Vec};
use stellar_macros::when_not_paused;

use super::EventContext;

use super::dust::require_no_dust_after;
use super::update;
use crate::cache::ControllerCache;
use crate::oracle::policy::OraclePolicy;
use crate::cross_contract::pool::pool_repay_call;
use crate::{storage, utils, validation, Controller, ControllerArgs, ControllerClient};

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
    let borrow_positions = storage::get_borrow_positions(env, account_id);
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
        process_single_repay(
            env,
            caller,
            account_id,
            &mut account,
            &asset,
            amount,
            &mut cache,
        );
    }

    // Partial repay must stay above dust floor or repay in full.
    require_no_dust_after(env, &mut cache, &account);

    // Repay does not delete account storage.
    storage::set_borrow_positions(env, account_id, &account.borrow_positions);
    cache.flush_isolated_debts();
    cache.emit_position_batch(account_id, &account);
    cache.emit_market_batch();
}

fn process_single_repay(
    env: &Env,
    caller: &Address,
    account_id: u64,
    account: &mut Account,
    asset: &Address,
    amount: i128,
    cache: &mut ControllerCache,
) {
    validation::require_amount_positive(env, amount);

    let position = account
        .borrow_positions
        .get(asset.clone())
        .unwrap_or_else(|| panic_with_error!(env, CollateralError::PositionNotFound));
    let actual_received = transfer_repayment_to_pool(env, caller, asset, amount, cache);

    let price_wad = if account.is_isolated {
        cache.cached_price(asset).price_wad
    } else {
        0
    };
    let _ = execute_repayment(
        env,
        account,
        account_id,
        asset,
        EventContext {
            caller: caller.clone(),
            event_caller: caller.clone(),
            action: symbol_short!("repay"),
        },
        &position,
        price_wad,
        actual_received,
        cache,
    );
}

// Executes pool repay.
#[allow(clippy::too_many_arguments)]
pub fn execute_repayment(
    env: &Env,
    account: &mut Account,
    _account_id: u64,
    asset: &Address,
    ctx: EventContext,
    position: &AccountPosition,
    price_wad: i128,
    amount: i128,
    cache: &mut ControllerCache,
) -> PoolPositionMutation {
    let EventContext {
        caller,
        event_caller,
        action,
    } = ctx;

    let pool_addr = cache.cached_pool_address(asset);
    let result = pool_repay_call(env, &pool_addr, caller.clone(), amount, position.clone());
    cache.record_market_update_with_price(
        &result.market_state,
        if price_wad > 0 { Some(price_wad) } else { None },
    );

    update::update_or_remove_position(
        account,
        AccountPositionType::Borrow,
        asset,
        &result.position,
    );
    if account.is_isolated {
        let feed = cache.cached_price(asset);
        adjust_isolated_debt_for_repay(
            env,
            account,
            cache,
            result.actual_amount,
            price_wad,
            feed.asset_decimals,
        );
    }
    let _ = event_caller;
    cache.record_position_update(
        action,
        AccountPositionType::Borrow,
        asset,
        result.market_index.borrow_index_ray,
        result.actual_amount,
        &result.position,
        if price_wad > 0 { Some(price_wad) } else { None },
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
        market_index.borrow_index_ray,
        feed.asset_decimals,
    );
    adjust_isolated_debt_for_repay(
        env,
        account,
        cache,
        actual_amount,
        feed.price_wad,
        feed.asset_decimals,
    );
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
    borrow_index_ray: i128,
    asset_decimals: u32,
) -> i128 {
    Ray::from_raw(position.scaled_amount_ray)
        .mul(env, Ray::from_raw(borrow_index_ray))
        .to_asset(asset_decimals)
}

fn adjust_isolated_debt_for_repay(
    env: &Env,
    account: &Account,
    cache: &mut ControllerCache,
    actual_amount: i128,
    price_wad: i128,
    asset_decimals: u32,
) {
    if account.is_isolated && actual_amount > 0 {
        utils::adjust_isolated_debt_usd(
            env,
            account,
            actual_amount,
            &price_wad,
            asset_decimals,
            cache,
        );
    }
}

