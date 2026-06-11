//! Borrow and strategy-internal borrow flows.
//!
//! Borrows use `OraclePolicy::RiskIncreasing`, update scaled debt shares, and
//! increment isolated-debt counters when the account is isolated.

use common::errors::{CollateralError, EModeError};
use common::types::{
    Account, AccountPositionType, AssetConfig, AssetConfigRaw, DebtPosition, Payment, PoolAction,
    PriceFeed,
};
use soroban_sdk::{
    assert_with_error, contractimpl, panic_with_error, symbol_short, Address, Env, Map, Symbol, Vec,
};
use stellar_macros::when_not_paused;

use crate::cache::Cache;
use crate::cross_contract::pool::{pool_borrow_call, pool_create_strategy_call};
use crate::emode;
use crate::helpers::{require_no_borrow_dust_for_assets, update_or_remove_debt_position};
use crate::oracle::policy::OraclePolicy;
use crate::positions::isolated_debt::add_isolated_debt;
use crate::{storage, utils, validation, Controller, ControllerArgs, ControllerClient};

/// Result of a single pool borrow/strategy call, ready to be reflected onto
/// the account's borrow position.
struct BorrowUpdate {
    action: Symbol,
    index: i128,
    amount: i128,
    position: DebtPosition,
    price_wad: i128,
}

#[contractimpl]
impl Controller {
    #[when_not_paused]
    pub fn borrow(env: Env, caller: Address, account_id: u64, borrows: Vec<(Address, i128)>) {
        process_borrow(&env, &caller, account_id, &borrows);
    }
}

/// Creates strategy debt in the pool and returns the asset amount received.
pub fn create_borrow_strategy(
    env: &Env,
    cache: &mut Cache,
    account: &mut Account,
    debt_token: &Address,
    amount: i128,
) -> i128 {
    validation::require_market_active(env, cache, debt_token);

    let e_mode = emode::active_e_mode_category(env, account.e_mode_category_id);
    let debt_config = emode::effective_asset_config(env, account, debt_token, cache, &e_mode);
    let mut new_borrows = Vec::new(env);
    new_borrows.push_back((debt_token.clone(), amount));
    validate_siloed_borrow_set(env, cache, account, &new_borrows);
    validate_borrow_asset_preflight(env, cache, &debt_config, debt_token, account);

    let price_feed = cache.cached_price(debt_token);

    add_isolated_debt(env, cache, account, amount, &price_feed);

    let flash_fee = debt_config.flashloan_fee.apply_to(env, amount);
    let borrow_position = account.get_or_create_debt_position(debt_token);

    let pool_addr = cache.cached_pool_address();
    let action = PoolAction {
        caller: env.current_contract_address(),
        position: (&borrow_position).into(),
        amount,
        asset: debt_token.clone(),
    };
    let result =
        pool_create_strategy_call(env, &pool_addr, action, flash_fee, debt_config.borrow_cap);
    cache.record_market_update_with_price(&result.market_state, Some(price_feed.price.raw()));
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

/// Borrows one or more assets after LTV pre-checks and final health validation.
pub fn process_borrow(env: &Env, caller: &Address, account_id: u64, borrows: &Vec<Payment>) {
    caller.require_auth();
    validation::require_not_flash_loaning(env);

    let mut account = storage::get_account(env, account_id);

    validation::require_account_owner_match(env, &account, caller);

    let mut cache = Cache::new(env, OraclePolicy::RiskIncreasing);
    // Dedup once at the entrypoint so every downstream stage, including the
    // post-flight dust scope, sees one entry per asset.
    let borrow_plan = utils::aggregate_positive_payments(env, borrows);

    // prepare_borrow_plan prices each plan asset and require_within_ltv then
    // prices the full supply+borrow set — all before the HF-body prefetch in
    // calculate_account_totals_body can fire. Prefetch positions + plan assets
    // here so every downstream read hits the cache (plan assets are listed
    // explicitly: they may not be in borrow_positions yet).
    let mut priced_assets = account.supply_positions.keys();
    priced_assets.append(&account.borrow_positions.keys());
    priced_assets.append(&utils::plan_assets(env, &borrow_plan));
    crate::oracle::prefetch_redstone_feeds(&mut cache, &priced_assets);

    process_borrow_plan(env, caller, &mut account, &borrow_plan, &mut cache);

    // LTV gate runs post-pool so collateral and debt valuation reuse the market
    // indexes the pool borrow just wrote into the cache, sparing a redundant
    // get_sync_data read. A failure here panics and reverts the atomic tx.
    validation::require_within_ltv(env, &mut cache, &account);
    validation::require_healthy_account(env, &mut cache, &account);
    // Scope the dust gate to borrowed assets only: borrow never mutates supply,
    // so it must not be blocked by pre-existing positions that drifted under the floor.
    require_no_borrow_dust_for_assets(
        env,
        &mut cache,
        &account,
        &utils::plan_assets(env, &borrow_plan),
    );

    storage::set_debt_positions(env, account_id, &account.borrow_positions);
    cache.flush_isolated_debts();
    cache.emit_position_batch(account_id, &account);
    cache.emit_market_batch();
}

/// Executes a validated, deduplicated borrow plan (`aggregate_positive_payments`
/// output): mutates the account, calls pools, records mutations for events.
fn process_borrow_plan(
    env: &Env,
    caller: &Address,
    account: &mut Account,
    borrow_plan: &Vec<Payment>,
    cache: &mut Cache,
) {
    let e_mode = emode::active_e_mode_category(env, account.e_mode_category_id);

    // Reuse the same e-mode-adjusted config for validation and pool execution.
    let mut effective_configs: Map<Address, AssetConfigRaw> = Map::new(env);
    for (asset, _) in borrow_plan.iter() {
        let cfg = emode::effective_asset_config(env, account, &asset, cache, &e_mode);
        effective_configs.set(asset, (&cfg).into());
    }

    prepare_borrow_plan(env, account, borrow_plan, cache, &effective_configs);
    execute_borrow_plan(env, caller, account, borrow_plan, cache, &effective_configs);
}

fn validate_borrow_asset_preflight(
    env: &Env,
    cache: &mut Cache,
    asset_config: &AssetConfig,
    asset: &Address,
    account: &Account,
) {
    if account.is_isolated && !asset_config.can_borrow_in_isolation() {
        panic_with_error!(env, EModeError::NotBorrowableIsolation);
    }

    emode::validate_e_mode_asset(env, cache, account.e_mode_category_id, asset);
    emode::ensure_e_mode_compatible_with_asset(env, asset_config, account.e_mode_category_id);

    assert_with_error!(
        env,
        asset_config.is_borrowable,
        CollateralError::AssetNotBorrowable
    );
}

// Cheap pre-pool gates: emptiness, position limits, market-active, siloed set,
// per-asset borrowability, and isolated-debt ceilings. The LTV valuation runs
// post-pool in `require_within_ltv` to reuse the borrow's cached market index.
fn prepare_borrow_plan(
    env: &Env,
    account: &Account,
    assets: &Vec<Payment>,
    cache: &mut Cache,
    effective_configs: &Map<Address, AssetConfigRaw>,
) {
    validation::require_non_empty_payments(env, assets);

    validation::validate_bulk_position_limits(env, account, AccountPositionType::Borrow, assets);
    for (asset, _) in assets {
        validation::require_market_active(env, cache, &asset);
    }
    validate_siloed_borrow_set(env, cache, account, assets);

    for (asset, amount) in assets {
        let asset_config: AssetConfig =
            (&validation::expect_invariant(env, effective_configs.get(asset.clone()))).into();
        validate_borrow_asset_preflight(env, cache, &asset_config, &asset, account);

        let feed = cache.cached_price(&asset);
        add_isolated_debt(env, cache, account, amount, &feed);
    }
}

fn validate_siloed_borrow_set(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    new_borrows: &Vec<Payment>,
) {
    let mut final_assets: Vec<Address> = Vec::new(env);
    for asset in account.borrow_positions.keys() {
        utils::push_unique_address(&mut final_assets, asset);
    }
    for (asset, _) in new_borrows {
        utils::push_unique_address(&mut final_assets, asset);
    }

    if final_assets.len() <= 1 {
        return;
    }

    for asset in final_assets {
        let config = cache.cached_asset_config(&asset);
        assert_with_error!(
            env,
            !config.is_siloed_borrowing,
            CollateralError::NotBorrowableSiloed
        );
    }
}

fn execute_borrow_plan(
    env: &Env,
    caller: &Address,
    account: &mut Account,
    assets: &Vec<Payment>,
    cache: &mut Cache,
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
    cache: &mut Cache,
) {
    let borrow_position = account.get_or_create_debt_position(req.asset);

    let pool_addr = cache.cached_pool_address();
    let action = PoolAction {
        caller: caller.clone(),
        position: (&borrow_position).into(),
        amount: req.amount,
        asset: req.asset.clone(),
    };
    let result = pool_borrow_call(env, &pool_addr, action, req.config.borrow_cap);
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
    cache: &mut Cache,
) {
    cache.record_debt_position_update(
        update.action,
        asset,
        update.index,
        update.amount,
        &update.position,
        Some(update.price_wad),
    );
    update_or_remove_debt_position(account, asset, &update.position);
}
