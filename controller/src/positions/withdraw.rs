use common::errors::{CollateralError, GenericError};
use common::events::{emit_update_position, EventAccountPosition, UpdatePositionEvent};
use common::types::{
    Account, AccountPosition, AccountPositionType, ControllerKey, Payment, PoolPositionMutation,
};
use soroban_sdk::{contractimpl, panic_with_error, symbol_short, Address, Env, Map, Vec};
use stellar_macros::when_not_paused;

use super::EventContext;

use super::update;
use crate::cache::ControllerCache;
use crate::{storage, utils, validation, Controller, ControllerArgs, ControllerClient};

/// Sentinel passed to the pool to request a full-position withdrawal. The
/// pool clamps any value `>= current_supply_actual` to the post-accrual
/// balance, so `i128::MAX` is the canonical "withdraw all" signal.
const WITHDRAW_ALL_SENTINEL: i128 = i128::MAX;

#[contractimpl]
impl Controller {
    #[when_not_paused]
    pub fn withdraw(env: Env, caller: Address, account_id: u64, withdrawals: Vec<Payment>) {
        process_withdraw(&env, &caller, account_id, &withdrawals);
    }
}

/// Processes a withdrawal batch and removes the account when all positions close.
///
/// Storage I/O:
///   * debt-free: 1 meta read + 1 supply-side read + 1 supply-side write
///     (or full account close if both sides become empty).
///   * with debt: + 1 borrow-side read for the post-batch HF gate.
pub fn process_withdraw(env: &Env, caller: &Address, account_id: u64, withdrawals: &Vec<Payment>) {
    caller.require_auth();
    validation::require_not_flash_loaning(env);

    let meta = storage::get_account_meta(env, account_id);
    let supply_positions = storage::get_supply_positions(env, account_id);

    // Supply-only exits are risk-decreasing; accounts with debt keep strict
    // oracle checks before the final health-factor gate. Probe the borrow
    // side key with `has` (cheap) to avoid loading the borrow map for
    // accounts that have no debt.
    let has_debt = env
        .storage()
        .persistent()
        .has(&ControllerKey::BorrowPositions(account_id));
    let borrow_positions = if has_debt {
        storage::get_borrow_positions(env, account_id)
    } else {
        Map::new(env)
    };

    let mut account = storage::account_from_parts(meta, supply_positions, borrow_positions);

    validation::require_account_owner_match(env, &account, caller);

    let allow_unsafe = account.borrow_positions.is_empty();
    let mut cache = ControllerCache::new(env, allow_unsafe);

    let withdrawal_plan = utils::aggregate_withdrawal_payments(env, withdrawals);
    for (asset, amount) in withdrawal_plan {
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

    validation::require_healthy_account(env, &mut cache, &account);

    if account.supply_positions.is_empty() && account.borrow_positions.is_empty() {
        utils::remove_account(env, account_id);
    } else {
        // Withdraw mutates only supply positions; borrow positions are read
        // only for health-factor validation.
        storage::set_supply_positions(env, account_id, &account.supply_positions);
    }
}

#[allow(clippy::too_many_arguments)]
fn process_single_withdrawal(
    env: &Env,
    caller: &Address,
    account_id: u64,
    account: &mut Account,
    asset: &Address,
    amount: i128,
    cache: &mut ControllerCache,
) {
    // `0` means withdraw all; negative withdrawals are never valid.
    if amount < 0 {
        panic_with_error!(env, GenericError::AmountMustBePositive);
    }

    let feed = cache.cached_price(asset);

    let position = match account.supply_positions.get(asset.clone()) {
        Some(pos) => pos,
        None => panic_with_error!(env, CollateralError::PositionNotFound),
    };

    let withdraw_amount = if amount == 0 {
        WITHDRAW_ALL_SENTINEL
    } else {
        amount
    };

    let _ = execute_withdrawal(
        env,
        account,
        account_id,
        asset,
        EventContext {
            caller: caller.clone(),
            event_caller: caller.clone(),
            action: symbol_short!("withdraw"),
        },
        withdraw_amount,
        &position,
        false,
        0,
        feed.price_wad,
        cache,
    );
}

/// Executes the pool withdrawal leg and records the account-side mutation.
/// `ctx.caller` receives tokens from the pool; `ctx.event_caller` is the
/// user address emitted for indexers.
#[allow(clippy::too_many_arguments)]
pub fn execute_withdrawal(
    env: &Env,
    account: &mut Account,
    account_id: u64,
    asset: &Address,
    ctx: EventContext,
    amount: i128,
    position: &AccountPosition,
    is_liquidation: bool,
    protocol_fee: i128,
    price_wad: i128,
    cache: &mut ControllerCache,
) -> PoolPositionMutation {
    let EventContext {
        caller,
        event_caller,
        action,
    } = ctx;
    let _ = cache;
    let result = pool_withdraw_call(
        env,
        asset,
        caller.clone(),
        amount,
        position.clone(),
        is_liquidation,
        protocol_fee,
        price_wad,
    );
    update::update_or_remove_position(
        account,
        AccountPositionType::Deposit,
        asset,
        &result.position,
    );

    emit_update_position(
        env,
        UpdatePositionEvent {
            action,
            index: result.market_index.supply_index_ray,
            amount: result.actual_amount,
            position: EventAccountPosition::new(
                AccountPositionType::Deposit,
                asset.clone(),
                account_id,
                &result.position,
            ),
            asset_price: Some(price_wad),
            caller: Some(event_caller),
            account_attributes: Some((&*account).into()),
        },
    );

    result
}

crate::summarized!(
    pool::withdraw_summary,
    fn pool_withdraw_call(
        env: &Env,
        asset: &Address,
        caller: Address,
        amount: i128,
        position: AccountPosition,
        is_liquidation: bool,
        protocol_fee: i128,
        price_wad: i128,
    ) -> PoolPositionMutation {
        let pool_addr = storage::get_market_config(env, asset).pool_address;
        pool_interface::LiquidityPoolClient::new(env, &pool_addr).withdraw(
            &caller,
            &amount,
            &position,
            &is_liquidation,
            &protocol_fee,
            &price_wad,
        )
    }
);
