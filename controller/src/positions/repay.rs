use common::errors::{CollateralError, GenericError};
use common::events::{emit_update_position, EventAccountPosition, UpdatePositionEvent};
use common::fp::Ray;
use common::types::{Account, AccountPosition, AccountPositionType, Payment, PoolPositionMutation};
use soroban_sdk::{contractimpl, panic_with_error, symbol_short, Address, Env, Map, Vec};
use stellar_macros::when_not_paused;

use super::EventContext;

use super::update;
use crate::cache::ControllerCache;
use crate::{storage, utils, validation, Controller, ControllerArgs, ControllerClient};

#[contractimpl]
impl Controller {
    #[when_not_paused]
    pub fn repay(env: Env, caller: Address, account_id: u64, payments: Vec<Payment>) {
        process_repay(&env, &caller, account_id, &payments);
    }
}

/// Processes a repayment batch. Any caller may repay any account.
///
/// Storage I/O: 1 meta read + 1 borrow-side read + 1 borrow-side write.
/// The supply side is never touched.
pub fn process_repay(env: &Env, caller: &Address, account_id: u64, payments: &Vec<Payment>) {
    caller.require_auth();
    validation::require_not_flash_loaning(env);
    validation::require_non_empty_payments(env, payments);

    let meta = storage::get_account_meta(env, account_id);
    let borrow_positions = storage::get_borrow_positions(env, account_id);
    // Isolated accounts must use safe prices: the per-repay decrement of
    // the global IsolatedDebt(asset) USD ceiling uses `feed.price_wad`, and a
    // stale price would drift the ceiling counter against actual debt. Other
    // (non-isolated) accounts have no such global accumulator, so a
    // permissive cache stays acceptable to keep repay reachable during
    // oracle degradation.
    let allow_unsafe = !meta.is_isolated;
    let mut account = storage::account_from_parts(meta, Map::new(env), borrow_positions);
    let mut cache = ControllerCache::new_with_disabled_market_price(env, allow_unsafe);

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

    // Full repay does not delete the account; only owner withdraw/close flows
    // may burn account storage. Meta is never mutated by repay; supply side
    // stays as it was on disk.
    storage::set_borrow_positions(env, account_id, &account.borrow_positions);
    cache.flush_isolated_debts();
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

    let feed = cache.cached_price(asset);
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
        feed.price_wad,
        actual_received,
        cache,
    );
}

/// Executes the pool repay leg and records the account-side mutation.
#[allow(clippy::too_many_arguments)]
pub fn execute_repayment(
    env: &Env,
    account: &mut Account,
    account_id: u64,
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

    let result = pool_repay_call(
        env,
        asset,
        caller.clone(),
        amount,
        position.clone(),
        price_wad,
    );

    update::update_or_remove_position(
        account,
        AccountPositionType::Borrow,
        asset,
        &result.position,
    );
    let feed = cache.cached_price(asset);
    adjust_isolated_debt_for_repay(
        env,
        account,
        cache,
        result.actual_amount,
        price_wad,
        feed.asset_decimals,
    );
    emit_update_position(
        env,
        UpdatePositionEvent {
            action,
            index: result.market_index.borrow_index_ray,
            amount: result.actual_amount,
            position: EventAccountPosition::new(
                AccountPositionType::Borrow,
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

/// Decrements isolated debt by the full current value of `position`.
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

crate::summarized!(
    pool::repay_summary,
    fn pool_repay_call(
        env: &Env,
        asset: &Address,
        caller: Address,
        amount: i128,
        position: AccountPosition,
        price_wad: i128,
    ) -> PoolPositionMutation {
        let pool_addr = crate::storage::get_market_config(env, asset).pool_address;
        pool_interface::LiquidityPoolClient::new(env, &pool_addr)
            .repay(&caller, &amount, &position, &price_wad)
    }
);
