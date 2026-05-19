use common::errors::{CollateralError, GenericError};
use common::types::{
    Account, AccountPosition, AccountPositionType, ControllerKey, Payment, PoolPositionMutation,
};
use soroban_sdk::{contractimpl, panic_with_error, symbol_short, Address, Env, Map, Vec};
use stellar_macros::when_not_paused;

use super::EventContext;

use super::dust::require_no_dust_after;
use super::update;
use crate::cache::ControllerCache;
use crate::oracle::policy::OraclePolicy;
use crate::cross_contract::pool::pool_withdraw_call;
use crate::{storage, utils, validation, Controller, ControllerArgs, ControllerClient};

// Sentinel for full-position withdraw.
const WITHDRAW_ALL_SENTINEL: i128 = i128::MAX;

#[contractimpl]
impl Controller {
    #[when_not_paused]
    pub fn withdraw(env: Env, caller: Address, account_id: u64, withdrawals: Vec<(Address, i128)>) {
        process_withdraw(&env, &caller, account_id, &withdrawals);
    }
}

// Processes withdraw batch.
pub fn process_withdraw(env: &Env, caller: &Address, account_id: u64, withdrawals: &Vec<Payment>) {
    caller.require_auth();
    validation::require_not_flash_loaning(env);

    let meta = storage::get_account_meta(env, account_id);
    let supply_positions = storage::get_supply_positions(env, account_id);

    // Supply-only exits are risk-decreasing.
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

    let policy = if account.borrow_positions.is_empty() {
        OraclePolicy::RiskDecreasing
    } else {
        OraclePolicy::RiskIncreasing
    };
    let mut cache = ControllerCache::new(env, policy);

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

    // Enforce HF and LTV gates.
    validation::require_within_ltv(env, &mut cache, &account);
    validation::require_healthy_account(env, &mut cache, &account);
    // Dust residue not allowed on partial withdraw.
    require_no_dust_after(env, &mut cache, &account);

    if account.supply_positions.is_empty() && account.borrow_positions.is_empty() {
        utils::remove_account(env, account_id);
    } else {
        // Mutates supply positions only.
        storage::set_supply_positions(env, account_id, &account.supply_positions);
    }
    cache.emit_position_batch(account_id, &account);
    cache.emit_market_batch();
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

// Executes pool withdraw.
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
    let pool_addr = cache.cached_pool_address(asset);
    let result = pool_withdraw_call(
        env,
        &pool_addr,
        caller.clone(),
        amount,
        position.clone(),
        is_liquidation,
        protocol_fee,
    );
    cache.record_market_update_with_price(&result.market_state, Some(price_wad));
    update::update_or_remove_position(
        account,
        AccountPositionType::Deposit,
        asset,
        &result.position,
    );

    let _ = env;
    let _ = account_id;
    let _ = event_caller;
    cache.record_position_update(
        action,
        AccountPositionType::Deposit,
        asset,
        result.market_index.supply_index_ray,
        result.actual_amount,
        &result.position,
        Some(price_wad),
    );

    result
}

