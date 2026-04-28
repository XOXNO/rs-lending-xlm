use common::constants::WAD;
use common::errors::{CollateralError, GenericError};
use common::events::{emit_update_position, UpdatePositionEvent};
use common::types::{Account, AccountPosition, PoolPositionMutation};
use soroban_sdk::{panic_with_error, symbol_short, Address, Env, Symbol, Vec};

use super::update;
use crate::cache::ControllerCache;
use crate::{helpers, storage, utils, validation};

/// Processes a batch of withdrawals. Validates ownership, applies a post-batch HF check
/// when borrows are open, and removes the account from storage when all positions close.
pub fn process_withdraw(
    env: &Env,
    caller: &Address,
    account_id: u64,
    withdrawals: &Vec<(Address, i128)>,
) {
    caller.require_auth();
    validation::require_not_paused(env);
    validation::require_not_flash_loaning(env);
    let mut account = storage::get_account(env, account_id);

    if account.owner != *caller {
        panic_with_error!(env, GenericError::AccountNotInMarket);
    }

    // Allow unsafe price only when the account has no debt: the post-loop
    // `validate_is_healthy` short-circuits when no borrows exist, so allowing
    // the unsafe price unlocks no risk-increasing operation. Supply-only
    // users can therefore exit during oracle deviation > 5%.
    let allow_unsafe = account.borrow_positions.is_empty();
    let mut cache = ControllerCache::new(env, allow_unsafe);

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

    // Post-batch health factor check; skip when no borrows exist.
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

    // When the account closes out, remove it entirely instead of rewriting an empty account.
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
    // Reject negative amounts. `amount == 0` is the "withdraw all" sentinel;
    // any negative value would otherwise reach `pool.withdraw` and, via
    // saturating_sub_ray on signed i128, mint phantom collateral.
    if amount < 0 {
        panic_with_error!(env, GenericError::AmountMustBePositive);
    }

    // Cache uses strict pricing whenever the account has any borrows,
    // matching the gate set in `process_withdraw`. With borrows present,
    // `oracle::token_price` blocks when deviation exceeds second tolerance.
    let feed = cache.cached_price(asset);

    let position = match account.supply_positions.get(asset.clone()) {
        Some(pos) => pos,
        None => panic_with_error!(env, CollateralError::PositionNotFound),
    };

    // `amount == 0` sentinel: withdraw all.
    let withdraw_amount = if amount == 0 { i128::MAX } else { amount };

    // Shared withdrawal execution (also used by liquidation and strategy flows).
    // The helper emits `UpdatePositionEvent` itself with the caller-provided
    // `action` tag, guaranteeing every position mutation produces an event
    // (plain / liquidation / strategy paths all covered).
    let _ = execute_withdrawal(
        env,
        account_id,
        account,
        caller,
        caller,
        symbol_short!("withdraw"),
        withdraw_amount,
        &position,
        false, // not liquidation
        0,     // no protocol fee
        feed.price_wad,
        cache,
    );
}

// ---------------------------------------------------------------------------
// Shared withdrawal execution (also used by liquidation)
// ---------------------------------------------------------------------------

/// Execute the withdrawal through the pool, update the position, and emit
/// an `UpdatePositionEvent`.
///
/// - `caller` is the pool-call authority (the address that receives tokens).
/// - `event_caller` is the originator logged in the event. For plain
///   withdraw and liquidation these are the same; for strategy flows (where
///   `caller = env.current_contract_address()` because the controller is
///   the intermediate recipient) pass the real user.
/// - `action` is the event tag the caller wants indexers to see
///   (e.g. `"withdraw"`, `"liq_seize"`, `"rp_col_wd"`, `"sw_col_wd"`,
///   `"close_wd"`).
#[allow(clippy::too_many_arguments)]
pub fn execute_withdrawal(
    env: &Env,
    _account_id: u64,
    account: &mut Account,
    caller: &Address,
    event_caller: &Address,
    action: Symbol,
    amount: i128,
    position: &AccountPosition,
    is_liquidation: bool,
    protocol_fee: i128,
    price_wad: i128,
    cache: &mut ControllerCache,
) -> PoolPositionMutation {
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

    // Withdraw uses supply_index_ray.
    emit_update_position(
        env,
        UpdatePositionEvent {
            action,
            index: result.market_index.supply_index_ray,
            amount: result.actual_amount,
            position: result.position.clone().into(),
            asset_price: Some(price_wad),
            caller: Some(event_caller.clone()),
            account_attributes: Some((&*account).into()),
        },
    );

    result
}
