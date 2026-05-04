use common::errors::{CollateralError, EModeError, GenericError};
use common::events::{emit_update_position, EventAccountPosition, UpdatePositionEvent};
use common::fp::{Bps, Ray, Wad};
use common::types::{
    Account, AccountPosition, AccountPositionType, AssetConfig, Payment, PriceFeed,
    POSITION_TYPE_BORROW,
};
use soroban_sdk::{contractimpl, panic_with_error, symbol_short, Address, Env, Map, Symbol, Vec};
use stellar_macros::when_not_paused;

use super::{emode, update};
use crate::cache::ControllerCache;
use crate::{helpers, storage, utils, validation, Controller, ControllerArgs, ControllerClient};

#[contractimpl]
impl Controller {
    #[when_not_paused]
    pub fn borrow(env: Env, caller: Address, account_id: u64, borrows: Vec<Payment>) {
        borrow_batch(&env, &caller, account_id, &borrows);
    }
}

/// Strategy borrow path: validates e-mode and borrowability, enforces the borrow cap and
/// isolated-debt ceiling, flashes the debt via `pool::create_strategy`, and emits the event.
/// Returns the net amount received after the pool deducts the strategy flash fee.
pub fn handle_create_borrow_strategy(
    env: &Env,
    cache: &mut ControllerCache,
    account: &mut Account,
    account_id: u64,
    debt_token: &Address,
    amount: i128,
    caller: &Address,
) -> i128 {
    validation::require_asset_supported(env, debt_token);
    validation::require_market_active(env, debt_token);

    let e_mode = emode::active_e_mode_category(env, account.e_mode_category_id);
    let debt_config = emode::effective_asset_config(env, account, debt_token, cache, &e_mode);
    let mut new_borrows = Vec::new(env);
    new_borrows.push_back((debt_token.clone(), amount));
    validate_siloed_borrow_set(env, cache, account, &new_borrows);
    validate_borrow_asset_preflight(env, &debt_config, debt_token, account);

    let price_feed = cache.cached_price(debt_token);

    validate_borrow_cap(env, cache, &debt_config, amount, debt_token);
    handle_isolated_debt(env, cache, account, amount, &price_feed);

    let flash_fee = Bps::from_raw(debt_config.flashloan_fee_bps).apply_to(env, amount);
    let borrow_position = get_or_create_borrow_position(account, &debt_config, debt_token);

    let result = pool_create_strategy_call(
        env,
        debt_token,
        env.current_contract_address(),
        borrow_position,
        amount,
        flash_fee,
        price_feed.price_wad,
    );
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
    );

    result.amount_received
}

// ---------------------------------------------------------------------------
// Batch entry point
// ---------------------------------------------------------------------------

/// Processes a batch of borrows: validates LTV collateral, enforces position limits,
/// and calls the pool for each asset. Post-batch HF gate prevents sub-threshold openings.
///
/// Storage I/O: 1 meta read + 1 supply-side read (LTV) + 1 borrow-side
/// read + 1 borrow-side write. The supply side and meta are not mutated.
pub fn borrow_batch(env: &Env, caller: &Address, account_id: u64, borrows: &Vec<Payment>) {
    caller.require_auth();
    validation::require_not_flash_loaning(env);

    let meta = storage::get_account_meta(env, account_id);
    let supply_positions = storage::get_supply_positions(env, account_id);
    let borrow_positions = storage::get_borrow_positions(env, account_id);
    let mut account = storage::account_from_parts(meta, supply_positions, borrow_positions);

    validation::require_account_owner_match(env, &account, caller);

    let mut cache = ControllerCache::new(env, false);
    process_borrow_plan(env, caller, account_id, &mut account, borrows, &mut cache);
    validation::require_healthy_account(env, &mut cache, &account);

    // Borrow only mutates the borrow side; supply and meta stay untouched.
    storage::set_borrow_positions(env, account_id, &account.borrow_positions);
    cache.flush_isolated_debts();
}

// ---------------------------------------------------------------------------
// process_borrow_plan -- reusable borrow flow
// ---------------------------------------------------------------------------

/// Processes a borrow batch on `account`: aggregates duplicate assets,
/// preflights the batch before pool mutation, then calls the pool once per
/// unique asset.
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

    // Resolve the e-mode-overlaid asset config once per unique asset; both
    // the preflight and the execute phase consume it without recomputing.
    let mut effective_configs: Map<Address, AssetConfig> = Map::new(env);
    for (asset, _) in borrow_plan.iter() {
        if !effective_configs.contains_key(asset.clone()) {
            let cfg = emode::effective_asset_config(env, account, &asset, cache, &e_mode);
            effective_configs.set(asset, cfg);
        }
    }

    prepare_borrow_plan(env, account, &borrow_plan, cache, &effective_configs);
    execute_borrow_plan(env, caller, account_id, account, &borrow_plan, cache, &effective_configs);
}

fn validate_borrow_asset_preflight(
    env: &Env,
    asset_config: &AssetConfig,
    asset: &Address,
    account: &Account,
) {
    if account.is_isolated && !asset_config.isolation_borrow_enabled {
        panic_with_error!(env, EModeError::NotBorrowableIsolation);
    }

    emode::validate_e_mode_asset(env, account.e_mode_category_id, asset, false);
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
        validation::require_asset_supported(env, &asset);
        validation::require_market_active(env, &asset);
    }
    validate_siloed_borrow_set(env, cache, account, assets);

    let ltv_collateral =
        helpers::calculate_ltv_collateral_wad(env, cache, &account.supply_positions).raw();
    let mut total_borrowed_wad = current_borrowed_wad(env, cache, &account.borrow_positions);

    for (asset, amount) in assets {
        let asset_config = effective_configs.get(asset.clone()).unwrap();
        validate_borrow_asset_preflight(env, &asset_config, &asset, account);

        let feed = cache.cached_price(&asset);
        total_borrowed_wad =
            validate_ltv_capacity(env, ltv_collateral, total_borrowed_wad, amount, &feed);
        validate_borrow_cap(env, cache, &asset_config, amount, &asset);
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
        let asset_config = effective_configs.get(asset.clone()).unwrap();
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
) {
    let borrow_position = get_or_create_borrow_position(account, asset_config, asset);

    let result = pool_borrow_call(
        env,
        asset,
        caller.clone(),
        amount,
        borrow_position,
        feed.price_wad,
    );

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
    );
}

crate::summarized!(
    crate::spec::summaries::pool::borrow_summary,
    fn pool_borrow_call(
        env: &Env,
        asset: &Address,
        caller: Address,
        amount: i128,
        position: AccountPosition,
        price_wad: i128,
    ) -> common::types::PoolPositionMutation {
        let pool_addr = storage::get_market_config(env, asset).pool_address;
        pool_interface::LiquidityPoolClient::new(env, &pool_addr).borrow(
            &caller,
            &amount,
            &position,
            &price_wad,
        )
    }
);

crate::summarized!(
    crate::spec::summaries::pool::create_strategy_summary,
    fn pool_create_strategy_call(
        env: &Env,
        asset: &Address,
        caller: Address,
        position: AccountPosition,
        amount: i128,
        fee: i128,
        price_wad: i128,
    ) -> common::types::PoolStrategyMutation {
        let pool_addr = storage::get_market_config(env, asset).pool_address;
        pool_interface::LiquidityPoolClient::new(env, &pool_addr).create_strategy(
            &caller,
            &position,
            &amount,
            &fee,
            &price_wad,
        )
    }
);

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
) {
    emit_update_position(
        env,
        UpdatePositionEvent {
            action,
            index,
            amount,
            position: EventAccountPosition::new(
                AccountPositionType::Borrow,
                asset.clone(),
                account_id,
                &position,
            ),
            asset_price: Some(price_wad),
            caller: Some(caller.clone()),
            account_attributes: Some((&*account).into()),
        },
    );
    update::update_or_remove_position(account, AccountPositionType::Borrow, asset, &position);
}

/// Increments the isolated-debt USD tracker by the USD value of `amount`.
/// Panics with `DebtCeilingReached` when the new total would exceed the isolation debt ceiling.
/// No-ops for non-isolated accounts.
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

fn validate_borrow_cap(
    env: &Env,
    cache: &mut ControllerCache,
    asset_config: &AssetConfig,
    amount: i128,
    asset: &Address,
) {
    if asset_config.borrow_cap == 0 {
        return; // Zero means no cap.
    }
    // Use the synced borrow index from the cache rather than the pool's stored
    // (potentially stale) value. `cached_market_index` simulates global_sync
    // forward to the current timestamp so cap enforcement is exact across
    // accrual gaps and same-tx multi-payment loops.
    let sync_data = cache.cached_pool_sync_data(asset);
    let market_index = cache.cached_market_index(asset);
    let current_total = Ray::from_raw(sync_data.state.borrowed_ray)
        .mul(env, Ray::from_raw(market_index.borrow_index_ray))
        .to_asset(sync_data.params.asset_decimals);
    let total = current_total
        .checked_add(amount)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    if total > asset_config.borrow_cap {
        panic_with_error!(env, CollateralError::BorrowCapReached);
    }
}

fn current_borrowed_wad(
    env: &Env,
    cache: &mut ControllerCache,
    borrow_positions: &Map<Address, AccountPosition>,
) -> i128 {
    let mut total_borrowed_wad: i128 = 0;
    for asset in borrow_positions.keys() {
        let position = borrow_positions.get(asset.clone()).unwrap();
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

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::helpers::testutils::test_market_config;
    use common::constants::RAY;
    use common::types::{MarketParams, PoolKey, PoolState, PositionMode};
    use soroban_sdk::testutils::{Address as _, Ledger, LedgerInfo};
    use soroban_sdk::{Address, Map};

    struct TestSetup {
        env: Env,
        controller: Address,
        asset: Address,
        other_asset: Address,
        pool: Address,
    }

    impl TestSetup {
        fn new() -> Self {
            let env = Env::default();
            env.mock_all_auths();
            env.ledger().set(LedgerInfo {
                timestamp: 1_000,
                protocol_version: 26,
                sequence_number: 100,
                network_id: Default::default(),
                base_reserve: 10,
                min_temp_entry_ttl: 10,
                min_persistent_entry_ttl: 10,
                max_entry_ttl: 3_110_400,
            });

            let admin = Address::generate(&env);
            let controller = env.register(crate::Controller, (admin.clone(),));
            let asset = env
                .register_stellar_asset_contract_v2(admin.clone())
                .address()
                .clone();
            let other_asset = Address::generate(&env);
            let params = MarketParams {
                max_borrow_rate_ray: 5 * RAY,
                base_borrow_rate_ray: RAY / 100,
                slope1_ray: RAY / 10,
                slope2_ray: RAY / 5,
                slope3_ray: RAY / 2,
                mid_utilization_ray: RAY / 2,
                optimal_utilization_ray: RAY * 8 / 10,
                reserve_factor_bps: 1_000,
                asset_id: asset.clone(),
                asset_decimals: 7,
            };
            let pool = env.register(pool::LiquidityPool, (controller.clone(), params));

            Self {
                env,
                controller,
                asset,
                other_asset,
                pool,
            }
        }

        fn as_controller<T>(&self, f: impl FnOnce() -> T) -> T {
            self.env.as_contract(&self.controller, f)
        }

        fn asset_config(&self, borrow_cap: i128, is_siloed_borrowing: bool) -> AssetConfig {
            AssetConfig {
                loan_to_value_bps: 7_500,
                liquidation_threshold_bps: 8_000,
                liquidation_bonus_bps: 500,
                liquidation_fees_bps: 100,
                is_collateralizable: true,
                is_borrowable: true,
                e_mode_enabled: false,
                is_isolated_asset: false,
                is_siloed_borrowing,
                is_flashloanable: true,
                isolation_borrow_enabled: true,
                isolation_debt_ceiling_usd_wad: 0,
                flashloan_fee_bps: 9,
                borrow_cap,
                supply_cap: i128::MAX,
            }
        }

        fn account_with_two_borrows(&self) -> Account {
            let mut borrow_positions = Map::new(&self.env);
            for asset in [&self.asset, &self.other_asset] {
                borrow_positions.set(
                    asset.clone(),
                    AccountPosition {
                        scaled_amount_ray: 1_0000000,
                        liquidation_threshold_bps: 8_000,
                        liquidation_bonus_bps: 500,
                        liquidation_fees_bps: 100,
                        loan_to_value_bps: 7_500,
                    },
                );
            }

            Account {
                owner: Address::generate(&self.env),
                is_isolated: false,
                e_mode_category_id: 0,
                mode: PositionMode::Normal,
                isolated_asset: None,
                supply_positions: Map::new(&self.env),
                borrow_positions,
            }
        }
    }

    #[test]
    #[should_panic]
    fn test_validate_borrow_cap_rejects_when_new_debt_exceeds_cap() {
        let t = TestSetup::new();

        t.as_controller(|| {
            t.env.as_contract(&t.pool, || {
                t.env.storage().instance().set(
                    &PoolKey::State,
                    &PoolState {
                        supplied_ray: 0,
                        borrowed_ray: 5 * RAY, // 5 tokens scaled to RAY (pool stores RAY-native)
                        revenue_ray: 0,
                        borrow_index_ray: RAY,
                        supply_index_ray: RAY,
                        last_timestamp: t.env.ledger().timestamp() * 1000,
                    },
                );
            });
            storage::set_market_config(
                &t.env,
                &t.asset,
                &test_market_config(&t.env, &t.asset, &t.pool, t.asset_config(5_0000000, false)),
            );

            let mut cache = ControllerCache::new(&t.env, true);
            validate_borrow_cap(
                &t.env,
                &mut cache,
                &t.asset_config(5_0000000, false),
                1_0000000,
                &t.asset,
            );
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #108)")]
    fn test_validate_borrow_asset_rejects_siloed_asset_on_multi_borrow_account() {
        let t = TestSetup::new();

        t.as_controller(|| {
            storage::set_market_config(
                &t.env,
                &t.other_asset,
                &test_market_config(&t.env, &t.asset, &t.pool, t.asset_config(0, false)),
            );
            storage::set_market_config(
                &t.env,
                &t.asset,
                &test_market_config(&t.env, &t.asset, &t.pool, t.asset_config(0, true)),
            );

            let mut cache = ControllerCache::new(&t.env, true);
            let account = t.account_with_two_borrows();
            let new_borrows = soroban_sdk::vec![&t.env, (t.asset.clone(), 1_0000000)];

            validate_siloed_borrow_set(&t.env, &mut cache, &account, &new_borrows);
        });
    }
}
