use common::errors::{CollateralError, EModeError, GenericError};
use common::fp::{Bps, Ray, Wad};
use common::types::{
    Account, AccountPosition, AccountPositionType, AssetConfig, Payment, PriceFeed,
    POSITION_TYPE_BORROW,
};
use soroban_sdk::{contractimpl, panic_with_error, symbol_short, Address, Env, Map, Symbol, Vec};
use stellar_macros::when_not_paused;

use super::dust::require_no_dust_after;
use super::{emode, update};
use crate::cache::ControllerCache;
use crate::oracle::policy::OraclePolicy;
use crate::cross_contract::pool::{pool_borrow_call, pool_create_strategy_call};
use crate::{helpers, storage, utils, validation, Controller, ControllerArgs, ControllerClient};

#[contractimpl]
impl Controller {
    #[when_not_paused]
    pub fn borrow(env: Env, caller: Address, account_id: u64, borrows: Vec<(Address, i128)>) {
        borrow_batch(&env, &caller, account_id, &borrows);
    }
}

// Strategy borrow path.
pub fn handle_create_borrow_strategy(
    env: &Env,
    cache: &mut ControllerCache,
    account: &mut Account,
    account_id: u64,
    debt_token: &Address,
    amount: i128,
    caller: &Address,
) -> i128 {
    validation::require_market_active(env, cache, debt_token);

    let e_mode = emode::active_e_mode_category(env, account.e_mode_category_id);
    let debt_config = emode::effective_asset_config(env, account, debt_token, cache, &e_mode);
    let mut new_borrows = Vec::new(env);
    new_borrows.push_back((debt_token.clone(), amount));
    validate_siloed_borrow_set(env, cache, account, &new_borrows);
    validate_borrow_asset_preflight(env, cache, &debt_config, debt_token, account);

    let price_feed = cache.cached_price(debt_token);

    handle_isolated_debt(env, cache, account, amount, &price_feed);

    let flash_fee = Bps::from_raw(debt_config.flashloan_fee_bps).apply_to(env, amount);
    let borrow_position = get_or_create_borrow_position(account, &debt_config, debt_token);

    let pool_addr = cache.cached_pool_address(debt_token);
    let result = pool_create_strategy_call(
        env,
        &pool_addr,
        env.current_contract_address(),
        borrow_position,
        amount,
        flash_fee,
        debt_config.borrow_cap,
    );
    cache.record_market_update_with_price(&result.market_state, Some(price_feed.price_wad));
    record_borrow_update(
        env,
        account,
        account_id,
        debt_token,
        symbol_short!("multiply"),
        result.market_index.borrow_index_ray,
        result.actual_amount,
        result.position,
        price_feed.price_wad,
        caller,
        cache,
    );

    result.amount_received
}



// Processes borrow batch.
pub fn borrow_batch(env: &Env, caller: &Address, account_id: u64, borrows: &Vec<Payment>) {
    caller.require_auth();
    validation::require_not_flash_loaning(env);

    let meta = storage::get_account_meta(env, account_id);
    let supply_positions = storage::get_supply_positions(env, account_id);
    let borrow_positions = storage::get_borrow_positions(env, account_id);
    let mut account = storage::account_from_parts(meta, supply_positions, borrow_positions);

    validation::require_account_owner_match(env, &account, caller);

    let mut cache = ControllerCache::new(env, OraclePolicy::RiskIncreasing);
    process_borrow_plan(env, caller, account_id, &mut account, borrows, &mut cache);
    validation::require_healthy_account(env, &mut cache, &account);
    // Rejects dust positions.
    require_no_dust_after(env, &mut cache, &account);

    // Mutates borrow positions only.
    storage::set_borrow_positions(env, account_id, &account.borrow_positions);
    cache.flush_isolated_debts();
    cache.emit_position_batch(account_id, &account);
    cache.emit_market_batch();
}



// Processes borrow plan on account.
pub fn process_borrow_plan(
    env: &Env,
    caller: &Address,
    account_id: u64,
    account: &mut Account,
    borrows: &Vec<Payment>,
    cache: &mut ControllerCache,
) {
    let e_mode = emode::active_e_mode_category(env, account.e_mode_category_id);
    let borrow_plan = utils::aggregate_positive_payments(env, borrows);

    // Resolve effective asset configs.
    let mut effective_configs: Map<Address, AssetConfig> = Map::new(env);
    for (asset, _) in borrow_plan.iter() {
        if !effective_configs.contains_key(asset.clone()) {
            let cfg = emode::effective_asset_config(env, account, &asset, cache, &e_mode);
            effective_configs.set(asset, cfg);
        }
    }

    prepare_borrow_plan(env, account, &borrow_plan, cache, &effective_configs);
    execute_borrow_plan(
        env,
        caller,
        account_id,
        account,
        &borrow_plan,
        cache,
        &effective_configs,
    );
}

fn validate_borrow_asset_preflight(
    env: &Env,
    cache: &mut ControllerCache,
    asset_config: &AssetConfig,
    asset: &Address,
    account: &Account,
) {
    if account.is_isolated && !asset_config.isolation_borrow_enabled {
        panic_with_error!(env, EModeError::NotBorrowableIsolation);
    }

    emode::validate_e_mode_asset(env, cache, account.e_mode_category_id, asset, false);
    emode::ensure_e_mode_compatible_with_asset(env, asset_config, account.e_mode_category_id);

    if !asset_config.is_borrowable {
        panic_with_error!(env, CollateralError::AssetNotBorrowable);
    }
}

fn prepare_borrow_plan(
    env: &Env,
    account: &Account,
    assets: &Vec<Payment>,
    cache: &mut ControllerCache,
    effective_configs: &Map<Address, AssetConfig>,
) {
    validation::require_non_empty_payments(env, assets);

    validation::validate_bulk_position_limits(env, account, POSITION_TYPE_BORROW, assets);
    for (asset, _) in assets {
        validation::require_market_active(env, cache, &asset);
    }
    validate_siloed_borrow_set(env, cache, account, assets);

    let ltv_collateral =
        helpers::calculate_ltv_collateral_wad(env, cache, &account.supply_positions).raw();
    let mut total_borrowed_wad = current_borrowed_wad(env, cache, &account.borrow_positions);

    for (asset, amount) in assets {
        let asset_config = validation::expect_invariant(env, effective_configs.get(asset.clone()));
        validate_borrow_asset_preflight(env, cache, &asset_config, &asset, account);

        let feed = cache.cached_price(&asset);
        total_borrowed_wad =
            validate_ltv_capacity(env, ltv_collateral, total_borrowed_wad, amount, &feed);
        handle_isolated_debt(env, cache, account, amount, &feed);
    }
}

fn validate_siloed_borrow_set(
    env: &Env,
    cache: &mut ControllerCache,
    account: &Account,
    new_borrows: &Vec<Payment>,
) {
    let mut final_assets: Vec<Address> = Vec::new(env);
    for asset in account.borrow_positions.keys() {
        push_unique_asset(&mut final_assets, asset);
    }
    for (asset, _) in new_borrows {
        push_unique_asset(&mut final_assets, asset);
    }

    if final_assets.len() <= 1 {
        return;
    }

    for asset in final_assets {
        let config = cache.cached_asset_config(&asset);
        if config.is_siloed_borrowing {
            panic_with_error!(env, CollateralError::NotBorrowableSiloed);
        }
    }
}

fn push_unique_asset(assets: &mut Vec<Address>, asset: Address) {
    if !assets.contains(asset.clone()) {
        assets.push_back(asset);
    }
}

fn execute_borrow_plan(
    env: &Env,
    caller: &Address,
    account_id: u64,
    account: &mut Account,
    assets: &Vec<Payment>,
    cache: &mut ControllerCache,
    effective_configs: &Map<Address, AssetConfig>,
) {
    for (asset, amount) in assets {
        let asset_config = validation::expect_invariant(env, effective_configs.get(asset.clone()));
        let feed = cache.cached_price(&asset);

        update_borrow_position(
            env,
            account_id,
            account,
            &asset,
            amount,
            &asset_config,
            caller,
            &feed,
            asset_config.borrow_cap,
            cache,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn update_borrow_position(
    env: &Env,
    account_id: u64,
    account: &mut Account,
    asset: &Address,
    amount: i128,
    asset_config: &AssetConfig,
    caller: &Address,
    feed: &PriceFeed,
    borrow_cap: i128,
    cache: &mut ControllerCache,
) {
    let borrow_position = get_or_create_borrow_position(account, asset_config, asset);

    let pool_addr = cache.cached_pool_address(asset);
    let result = pool_borrow_call(
        env,
        &pool_addr,
        caller.clone(),
        amount,
        borrow_position,
        borrow_cap,
    );
    cache.record_market_update_with_price(&result.market_state, Some(feed.price_wad));

    record_borrow_update(
        env,
        account,
        account_id,
        asset,
        symbol_short!("borrow"),
        result.market_index.borrow_index_ray,
        result.actual_amount,
        result.position,
        feed.price_wad,
        caller,
        cache,
    );
}

#[allow(clippy::too_many_arguments)]
fn record_borrow_update(
    env: &Env,
    account: &mut Account,
    account_id: u64,
    asset: &Address,
    action: Symbol,
    index: i128,
    amount: i128,
    position: AccountPosition,
    price_wad: i128,
    caller: &Address,
    cache: &mut ControllerCache,
) {
    let _ = env;
    let _ = account_id;
    let _ = caller;
    cache.record_position_update(
        action,
        AccountPositionType::Borrow,
        asset,
        index,
        amount,
        &position,
        Some(price_wad),
    );
    update::update_or_remove_position(account, AccountPositionType::Borrow, asset, &position);
}

// Increments isolated-debt tracker and checks ceiling.
pub fn handle_isolated_debt(
    env: &Env,
    cache: &mut ControllerCache,
    account: &Account,
    amount: i128,
    feed: &PriceFeed,
) {
    if !account.is_isolated {
        return;
    }

    let amount_wad = Wad::from_token(amount, feed.asset_decimals);
    let amount_in_usd_wad = amount_wad.mul(env, Wad::from_raw(feed.price_wad)).raw();

    let isolated_token = account
        .try_isolated_token()
        .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError));
    let collateral_config = cache.cached_asset_config(&isolated_token);

    // Read current debt from the cache to stay consistent with pending
    // in-batch deltas and with repay.rs's adjust_isolated_debt_usd path.
    let current_debt = cache.get_isolated_debt(&isolated_token);
    let new_debt = current_debt
        .checked_add(amount_in_usd_wad)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));

    if new_debt > collateral_config.isolation_debt_ceiling_usd_wad {
        panic_with_error!(env, EModeError::DebtCeilingReached);
    }

    // Write back through the cache; flush defers the storage write and emits
    // a single `UpdateDebtCeilingEvent` per asset at end-of-batch
    // (`ControllerCache::flush_isolated_debts`). Emitting here too would
    // produce one event per in-batch borrow against the same isolated asset.
    cache.set_isolated_debt(&isolated_token, new_debt);
}

fn get_or_create_borrow_position(
    account: &Account,
    borrow_asset_config: &AssetConfig,
    asset: &Address,
) -> AccountPosition {
    account
        .borrow_positions
        .get(asset.clone())
        .unwrap_or(AccountPosition {
            scaled_amount_ray: 0,
            liquidation_threshold_bps: borrow_asset_config.liquidation_threshold_bps,
            liquidation_bonus_bps: borrow_asset_config.liquidation_bonus_bps,
            liquidation_fees_bps: borrow_asset_config.liquidation_fees_bps,
            loan_to_value_bps: borrow_asset_config.loan_to_value_bps,
        })
}

fn current_borrowed_wad(
    env: &Env,
    cache: &mut ControllerCache,
    borrow_positions: &Map<Address, AccountPosition>,
) -> i128 {
    let mut total_borrowed_wad: i128 = 0;
    for asset in borrow_positions.keys() {
        let position = validation::expect_invariant(env, borrow_positions.get(asset.clone()));
        let position_feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);

        let actual = Ray::from_raw(position.scaled_amount_ray)
            .mul(env, Ray::from_raw(market_index.borrow_index_ray));
        let actual_wad = actual.to_wad();
        let value = actual_wad
            .mul(env, Wad::from_raw(position_feed.price_wad))
            .raw();
        total_borrowed_wad = total_borrowed_wad
            .checked_add(value)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    }
    total_borrowed_wad
}

fn validate_ltv_capacity(
    env: &Env,
    ltv_base_amount_wad: i128,
    borrowed_amount_wad: i128,
    amount: i128,
    feed: &PriceFeed,
) -> i128 {
    let amount_wad = Wad::from_token(amount, feed.asset_decimals);
    let new_borrow_wad = amount_wad.mul(env, Wad::from_raw(feed.price_wad)).raw();
    let total_borrow_wad = borrowed_amount_wad
        .checked_add(new_borrow_wad)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));

    if ltv_base_amount_wad < total_borrow_wad {
        panic_with_error!(env, CollateralError::InsufficientCollateral);
    }
    total_borrow_wad
}
