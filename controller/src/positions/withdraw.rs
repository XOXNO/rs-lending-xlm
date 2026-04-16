use common::constants::WAD;
use common::errors::CollateralError;
use common::events::{emit_update_position, UpdatePositionEvent};
use common::types::{Account, AccountPosition};
use soroban_sdk::{panic_with_error, symbol_short, Address, Env, Vec};

use super::update;
use crate::cache::ControllerCache;
use crate::{helpers, storage, utils, validation};

pub fn process_withdraw(
    env: &Env,
    caller: &Address,
    account_id: u64,
    withdrawals: &Vec<(Address, i128)>,
) {
    caller.require_auth();
    validation::require_not_paused(env);
    validation::require_not_flash_loaning(env);
    // Load account once — single storage read
    let mut account = storage::get_account(env, account_id);

    // Owner check
    if account.owner != *caller {
        panic_with_error!(env, common::errors::GenericError::AccountNotInMarket);
    }

    let mut cache = ControllerCache::new(env, false); // withdraw is risk-increasing

    for (asset, amount) in withdrawals {
        process_single_withdrawal(
            env,
            caller,
            account_id,
            &mut account,
            &asset,
            amount,
            &mut cache,
        );
    }

    // Post-batch health factor check (skip if no borrows)
    if !account.borrow_positions.is_empty() {
        let hf = helpers::calculate_health_factor(
            env,
            &mut cache,
            &account.supply_positions,
            &account.borrow_positions,
        );
        if hf < WAD {
            panic_with_error!(env, CollateralError::InsufficientCollateral);
        }
    }

    // Branches correctly: if closed out, remove entirely instead of rewriting empty account.
    if account.supply_positions.is_empty() && account.borrow_positions.is_empty() {
        utils::validate_account_is_empty(env, &account);
        utils::remove_account(env, account_id);
    } else {
        storage::set_account(env, account_id, &account);
    }
}

fn process_single_withdrawal(
    env: &Env,
    caller: &Address,
    account_id: u64,
    account: &mut Account,
    asset: &Address,
    amount: i128,
    cache: &mut ControllerCache,
) {
    // Price fetch — withdraw is risk-increasing so cache has allow_unsafe_price=false.
    // oracle::token_price() blocks automatically when deviation exceeds second tolerance.
    let feed = cache.cached_price(asset);

    // Position must exist
    let position = match account.supply_positions.get(asset.clone()) {
        Some(pos) => pos,
        None => panic_with_error!(env, CollateralError::PositionNotFound),
    };

    // 0 = withdraw all
    let withdraw_amount = if amount == 0 { i128::MAX } else { amount };

    // Shared withdrawal execution (also used by liquidation)
    let result = execute_withdrawal(
        env,
        account_id,
        account,
        caller,
        withdraw_amount,
        &position,
        false, // not liquidation
        0,     // no protocol fee
        feed.price_wad,
        cache,
    );

    // Withdraw uses supply_index_ray
    emit_update_position(
        env,
        UpdatePositionEvent {
            action: symbol_short!("withdraw"),
            index: result.market_index.supply_index_ray,
            amount: result.actual_amount,
            position: result.position.clone().into(),
            asset_price: Some(feed.price_wad),
            caller: Some(caller.clone()),
            account_attributes: Some((&*account).into()),
        },
    );
}

// ---------------------------------------------------------------------------
// Shared withdrawal execution (also used by liquidation)
// ---------------------------------------------------------------------------

pub fn execute_withdrawal(
    env: &Env,
    _account_id: u64,
    account: &mut Account,
    caller: &Address,
    amount: i128,
    position: &AccountPosition,
    is_liquidation: bool,
    protocol_fee: i128,
    price_wad: i128,
    cache: &mut ControllerCache,
) -> common::types::PoolPositionMutation {
    let pool_addr = cache.cached_pool_address(&position.asset);
    let pool_client = pool_interface::LiquidityPoolClient::new(env, &pool_addr);
    let result = pool_client.withdraw(
        caller,
        &amount,
        position,
        &is_liquidation,
        &protocol_fee,
        &price_wad,
    );
    update::update_or_remove_position(account, &result.position);
    result
}
