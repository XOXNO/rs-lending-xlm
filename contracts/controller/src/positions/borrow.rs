use common::errors::{CollateralError, EModeError, GenericError};
use common::types::{
    Account, AccountPosition, AccountPositionType, AssetConfig, AssetConfigRaw, Payment, PriceFeed,
};
use soroban_sdk::{contractimpl, panic_with_error, symbol_short, Address, Env, Map, Symbol, Vec};
use stellar_macros::when_not_paused;

use super::dust::require_no_dust_after;
use super::{emode, update};
use crate::cache::ControllerCache;
use crate::cross_contract::pool::{pool_borrow_call, pool_create_strategy_call};
use crate::oracle::policy::OraclePolicy;
use crate::{helpers, storage, utils, validation, Controller, ControllerArgs, ControllerClient};

/// Result of a single pool borrow/strategy call, ready to be reflected onto
/// the account's borrow position.
struct BorrowUpdate {
    action: Symbol,
    index: i128,
    amount: i128,
    position: AccountPosition,
    price_wad: i128,
}

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

    let flash_fee = debt_config.flashloan_fee.apply_to(env, amount);
    let borrow_position =
        account.get_or_create_position(AccountPositionType::Borrow, debt_token, &debt_config);

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
    cache.record_market_update_with_price(&result.market_state, Some(price_feed.price.raw()));
    let _ = account_id;
    let _ = caller;
    record_borrow_update(
        account,
        debt_token,
        BorrowUpdate {
            action: symbol_short!("multiply"),
            index: result.market_index.borrow_index_ray,
            amount: result.actual_amount,
            position: (&result.position).into(),
            price_wad: price_feed.price.raw(),
        },
        cache,
    );

    result.amount_received
}

// Processes borrow batch.
pub fn borrow_batch(env: &Env, caller: &Address, account_id: u64, borrows: &Vec<Payment>) {
    caller.require_auth();
    validation::require_not_flash_loaning(env);

    let meta = storage::get_account_meta(env, account_id);
    let supply_positions = storage::get_positions(env, account_id, AccountPositionType::Deposit);
    let borrow_positions = storage::get_positions(env, account_id, AccountPositionType::Borrow);
    let mut account = storage::account_from_parts(meta, supply_positions, borrow_positions);

    validation::require_account_owner_match(env, &account, caller);

    let mut cache = ControllerCache::new(env, OraclePolicy::RiskIncreasing);
    process_borrow_plan(env, caller, account_id, &mut account, borrows, &mut cache);
    validation::require_healthy_account(env, &mut cache, &account);
    // Rejects dust positions.
    require_no_dust_after(env, &mut cache, &account);

    // Mutates borrow positions only.
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
    let mut effective_configs: Map<Address, AssetConfigRaw> = Map::new(env);
    for (asset, _) in borrow_plan.iter() {
        if !effective_configs.contains_key(asset.clone()) {
            let cfg = emode::effective_asset_config(env, account, &asset, cache, &e_mode);
            effective_configs.set(asset, (&cfg).into());
        }
    }

    let _ = account_id;
    prepare_borrow_plan(env, account, &borrow_plan, cache, &effective_configs);
    execute_borrow_plan(
        env,
        caller,
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

// Batch-borrow risk gate (first of two — the strategy-borrow path in
// `handle_create_borrow_strategy` has its own checks and does not pass here).
//   1. here — per-asset cumulative LTV against pre-borrow collateral
//      (`validate_ltv_capacity`), so a borrow that would exceed LTV is
//      rejected before any pool call.
//   2. after `execute_borrow_plan`, in `borrow_batch` — final HF
//      (`require_healthy_account`) against the post-borrow positions,
//      catching any rounding the pre-check missed.
fn prepare_borrow_plan(
    env: &Env,
    account: &Account,
    assets: &Vec<Payment>,
    cache: &mut ControllerCache,
    effective_configs: &Map<Address, AssetConfigRaw>,
) {
    validation::require_non_empty_payments(env, assets);

    validation::validate_bulk_position_limits(env, account, AccountPositionType::Borrow, assets);
    for (asset, _) in assets {
        validation::require_market_active(env, cache, &asset);
    }
    validate_siloed_borrow_set(env, cache, account, assets);

    let ltv_collateral =
        helpers::calculate_ltv_collateral_wad(env, cache, &account.supply_positions).raw();
    let mut total_borrowed_wad =
        helpers::calculate_total_debt_wad(env, cache, &account.borrow_positions).raw();

    for (asset, amount) in assets {
        let asset_config: AssetConfig =
            (&validation::expect_invariant(env, effective_configs.get(asset.clone()))).into();
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
    account: &mut Account,
    assets: &Vec<Payment>,
    cache: &mut ControllerCache,
    effective_configs: &Map<Address, AssetConfigRaw>,
) {
    for (asset, amount) in assets {
        let asset_config: AssetConfig =
            (&validation::expect_invariant(env, effective_configs.get(asset.clone()))).into();
        let feed = cache.cached_price(&asset);

        update_borrow_position(
            env,
            account,
            BorrowRequest {
                asset: &asset,
                amount,
                config: &asset_config,
                feed: &feed,
            },
            caller,
            cache,
        );
    }
}

/// Inputs for a single borrow-position update.
struct BorrowRequest<'a> {
    asset: &'a Address,
    amount: i128,
    config: &'a AssetConfig,
    feed: &'a PriceFeed,
}

fn update_borrow_position(
    env: &Env,
    account: &mut Account,
    req: BorrowRequest<'_>,
    caller: &Address,
    cache: &mut ControllerCache,
) {
    let borrow_position =
        account.get_or_create_position(AccountPositionType::Borrow, req.asset, req.config);

    let pool_addr = cache.cached_pool_address(req.asset);
    let result = pool_borrow_call(
        env,
        &pool_addr,
        caller.clone(),
        req.amount,
        borrow_position,
        req.config.borrow_cap,
    );
    cache.record_market_update_with_price(&result.market_state, Some(req.feed.price.raw()));

    record_borrow_update(
        account,
        req.asset,
        BorrowUpdate {
            action: symbol_short!("borrow"),
            index: result.market_index.borrow_index_ray,
            amount: result.actual_amount,
            position: (&result.position).into(),
            price_wad: req.feed.price.raw(),
        },
        cache,
    );
}

fn record_borrow_update(
    account: &mut Account,
    asset: &Address,
    update: BorrowUpdate,
    cache: &mut ControllerCache,
) {
    cache.record_position_update(
        update.action,
        AccountPositionType::Borrow,
        asset,
        update.index,
        update.amount,
        &update.position,
        Some(update.price_wad),
    );
    update::update_or_remove_position(
        account,
        AccountPositionType::Borrow,
        asset,
        &update.position,
    );
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

    let amount_in_usd_wad = feed.usd_value_wad(env, amount).raw();

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

    if new_debt > collateral_config.isolation_debt_ceiling_usd.raw() {
        panic_with_error!(env, EModeError::DebtCeilingReached);
    }

    // Write back through the cache; flush defers the storage write and emits
    // a single `UpdateDebtCeilingEvent` per asset at end-of-batch
    // (`ControllerCache::flush_isolated_debts`). Emitting here too would
    // produce one event per in-batch borrow against the same isolated asset.
    cache.set_isolated_debt(&isolated_token, new_debt);
}

fn validate_ltv_capacity(
    env: &Env,
    ltv_base_amount_wad: i128,
    borrowed_amount_wad: i128,
    amount: i128,
    feed: &PriceFeed,
) -> i128 {
    let new_borrow_wad = feed.usd_value_wad(env, amount).raw();
    let total_borrow_wad = borrowed_amount_wad
        .checked_add(new_borrow_wad)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));

    if ltv_base_amount_wad < total_borrow_wad {
        panic_with_error!(env, CollateralError::InsufficientCollateral);
    }
    total_borrow_wad
}
