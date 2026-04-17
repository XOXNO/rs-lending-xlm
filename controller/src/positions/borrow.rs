use common::errors::{CollateralError, EModeError, GenericError};
use common::events::{
    emit_update_debt_ceiling, emit_update_position, UpdateDebtCeilingEvent, UpdatePositionEvent,
};
use common::fp::{Bps, Ray, Wad};
use common::types::{Account, AccountPosition, AssetConfig, PriceFeed, POSITION_TYPE_BORROW};
use soroban_sdk::{panic_with_error, symbol_short, Address, Env, Map, Vec};

use super::{emode, update};
use crate::cache::ControllerCache;
use crate::{helpers, storage, validation};

// ---------------------------------------------------------------------------
// handle_create_borrow_strategy
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub fn handle_create_borrow_strategy(
    env: &Env,
    cache: &mut ControllerCache,
    account: &mut Account,
    account_id: u64,
    debt_token: &Address,
    amount: i128,
    debt_config: &mut AssetConfig,
    caller: &Address,
) -> i128 {
    validation::require_asset_supported(env, debt_token);

    // Validate and override e-mode.
    let e_mode = emode::e_mode_category(env, account.e_mode_category_id);
    emode::ensure_e_mode_not_deprecated(env, &e_mode);
    let debt_emode_config = emode::token_e_mode_config(env, account.e_mode_category_id, debt_token);
    emode::ensure_e_mode_compatible_with_asset(env, debt_config, account.e_mode_category_id);
    emode::apply_e_mode_to_asset_config(env, debt_config, &e_mode, debt_emode_config);

    if !debt_config.is_borrowable {
        panic_with_error!(env, CollateralError::AssetNotBorrowable);
    }

    let borrow_position =
        get_or_create_borrow_position(account, account_id, debt_config, debt_token);
    let price_feed = cache.cached_price(debt_token);

    validate_borrow_cap(
        env,
        cache,
        debt_config,
        amount,
        price_feed.asset_decimals,
        debt_token,
    );

    handle_isolated_debt(env, cache, account, amount, &price_feed);

    let flash_fee = Bps::from_raw(debt_config.flashloan_fee_bps).apply_to(env, amount);

    let pool_address = cache.cached_pool_address(debt_token);

    validate_borrow_asset(env, cache, debt_config, debt_token, account);

    let pool_client = pool_interface::LiquidityPoolClient::new(env, &pool_address);
    let result = pool_client.create_strategy(
        &env.current_contract_address(),
        &borrow_position,
        &amount,
        &flash_fee,
        &price_feed.price_wad,
    );
    let mut updated_borrow_position = result.position;
    updated_borrow_position.account_id = account_id;

    emit_update_position(
        env,
        UpdatePositionEvent {
            action: symbol_short!("multiply"),
            index: result.market_index.borrow_index_ray,
            amount: result.actual_amount,
            position: updated_borrow_position.clone().into(),
            asset_price: Some(price_feed.price_wad),
            caller: Some(caller.clone()),
            account_attributes: Some((&*account).into()),
        },
    );

    // Persist the updated position on the in-memory account. The strategy
    // caller writes once via storage::set_account.
    update::update_or_remove_position(account, &updated_borrow_position);

    result.amount_received
}

// ---------------------------------------------------------------------------
// Batch entry point
// ---------------------------------------------------------------------------

pub fn borrow_batch(env: &Env, caller: &Address, account_id: u64, borrows: &Vec<(Address, i128)>) {
    caller.require_auth();
    validation::require_not_paused(env);
    validation::require_not_flash_loaning(env);
    let mut account = storage::get_account(env, account_id);

    if account.owner != *caller {
        panic_with_error!(env, GenericError::AccountNotInMarket);
    }

    // Block new borrows in a deprecated e-mode category. Otherwise a user
    // whose stored `loan_to_value_bps` still reflects the boosted e-mode
    // cap could borrow against that inflated value after deprecation,
    // draining the `(e_mode_ltv - base_ltv) * collateral` slack as bad debt.
    let e_mode = emode::e_mode_category(env, account.e_mode_category_id);
    emode::ensure_e_mode_not_deprecated(env, &e_mode);

    // Pre-flight position limit check rejects the full batch atomically.
    validation::validate_bulk_position_limits(env, &account, POSITION_TYPE_BORROW, borrows);

    let mut cache = ControllerCache::new(env, false);

    // Pre-compute LTV-weighted collateral ONCE and reuse across iterations.
    let ltv_collateral =
        helpers::calculate_ltv_collateral_wad(env, &mut cache, &account.supply_positions).raw();

    for i in 0..borrows.len() {
        let (asset, amount) = borrows.get(i).unwrap();
        process_borrow(
            env,
            &mut cache,
            account_id,
            caller,
            &mut account,
            &asset,
            amount,
            ltv_collateral,
        );
    }

    // Single storage write at the end of the batch.
    storage::set_account(env, account_id, &account);
    cache.flush_isolated_debts();
}

// ---------------------------------------------------------------------------
// handle_borrow_position
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn handle_borrow_position(
    env: &Env,
    cache: &mut ControllerCache,
    account_id: u64,
    asset: &Address,
    amount: i128,
    caller: &Address,
    account: &Account,
    borrow_position: AccountPosition,
    feed: &PriceFeed,
) -> AccountPosition {
    let mut borrow_position = borrow_position;
    borrow_position.account_id = account_id;

    let pool_address = cache.cached_pool_address(asset);

    let result = execute_borrow(
        env,
        &pool_address,
        caller,
        amount,
        &borrow_position,
        feed.price_wad,
    );

    let updated_position = result.position;

    emit_update_position(
        env,
        UpdatePositionEvent {
            action: symbol_short!("borrow"),
            index: result.market_index.borrow_index_ray,
            amount: result.actual_amount,
            position: updated_position.clone().into(),
            asset_price: Some(feed.price_wad),
            caller: Some(caller.clone()),
            account_attributes: Some(account.into()),
        },
    );

    updated_position
}

// ---------------------------------------------------------------------------
// execute_borrow
// ---------------------------------------------------------------------------

fn execute_borrow(
    env: &Env,
    pool_address: &Address,
    caller: &Address,
    amount: i128,
    position: &AccountPosition,
    price_wad: i128,
) -> common::types::PoolPositionMutation {
    let pool_client = pool_interface::LiquidityPoolClient::new(env, pool_address);
    pool_client.borrow(caller, &amount, position, &price_wad)
}

// ---------------------------------------------------------------------------
// handle_isolated_debt
// ---------------------------------------------------------------------------

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
    let new_debt = current_debt + amount_in_usd_wad;

    if new_debt > collateral_config.isolation_debt_ceiling_usd_wad {
        panic_with_error!(env, EModeError::DebtCeilingReached);
    }

    // Write back through the cache; flush defers the storage write and event.
    cache.set_isolated_debt(&isolated_token, new_debt);

    emit_update_debt_ceiling(
        env,
        UpdateDebtCeilingEvent {
            asset: isolated_token,
            total_debt_usd_wad: new_debt,
        },
    );
}

// ---------------------------------------------------------------------------
// get_or_create_borrow_position
// ---------------------------------------------------------------------------

fn get_or_create_borrow_position(
    account: &Account,
    account_id: u64,
    borrow_asset_config: &AssetConfig,
    asset: &Address,
) -> AccountPosition {
    account
        .borrow_positions
        .get(asset.clone())
        .unwrap_or_else(|| AccountPosition {
            position_type: common::types::AccountPositionType::Borrow,
            asset: asset.clone(),
            scaled_amount_ray: 0,
            account_id,
            liquidation_threshold_bps: borrow_asset_config.liquidation_threshold_bps,
            liquidation_bonus_bps: borrow_asset_config.liquidation_bonus_bps,
            liquidation_fees_bps: borrow_asset_config.liquidation_fees_bps,
            loan_to_value_bps: borrow_asset_config.loan_to_value_bps,
        })
}

// ---------------------------------------------------------------------------
// validate_borrow_cap
// ---------------------------------------------------------------------------

fn validate_borrow_cap(
    env: &Env,
    cache: &mut ControllerCache,
    asset_config: &AssetConfig,
    amount: i128,
    _asset_decimals: u32,
    asset: &Address,
) {
    if asset_config.borrow_cap == 0 {
        return; // Zero means no cap.
    }
    let pool_addr = cache.cached_pool_address(asset);
    let pool_client = pool_interface::LiquidityPoolClient::new(env, &pool_addr);
    let current_borrowed = pool_client.borrowed_amount(); // Returns asset decimals.
    if current_borrowed.saturating_add(amount) > asset_config.borrow_cap {
        panic_with_error!(env, CollateralError::BorrowCapReached);
    }
}

// ---------------------------------------------------------------------------
// validate_borrow_collateral
// ---------------------------------------------------------------------------

fn validate_borrow_collateral(
    env: &Env,
    ltv_base_amount_wad: i128,
    borrowed_amount_wad: i128,
    amount_to_borrow_wad: i128,
) {
    if ltv_base_amount_wad < borrowed_amount_wad + amount_to_borrow_wad {
        panic_with_error!(env, CollateralError::InsufficientCollateral);
    }
}

// ---------------------------------------------------------------------------
// validate_ltv_collateral
// ---------------------------------------------------------------------------

fn validate_ltv_collateral(
    env: &Env,
    cache: &mut ControllerCache,
    ltv_base_amount_wad: i128,
    borrow_positions: &Map<Address, AccountPosition>,
    amount: i128,
    feed: &PriceFeed,
) {
    // Total existing borrows in WAD USD. Iterated each call because borrows mutate.
    let mut total_borrowed_wad: i128 = 0;
    for asset in borrow_positions.keys() {
        let position = borrow_positions.get(asset.clone()).unwrap();
        let position_feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);

        let actual = Ray::from_raw(position.scaled_amount_ray)
            .mul(env, Ray::from_raw(market_index.borrow_index_ray));
        let actual_wad = actual.to_wad();
        total_borrowed_wad += actual_wad
            .mul(env, Wad::from_raw(position_feed.price_wad))
            .raw();
    }

    let amount_wad = Wad::from_token(amount, feed.asset_decimals);
    let new_borrow_wad = amount_wad.mul(env, Wad::from_raw(feed.price_wad)).raw();

    validate_borrow_collateral(env, ltv_base_amount_wad, total_borrowed_wad, new_borrow_wad);
}

// ---------------------------------------------------------------------------
// validate_borrow_asset
// ---------------------------------------------------------------------------

fn validate_borrow_asset(
    env: &Env,
    cache: &mut ControllerCache,
    asset_config: &AssetConfig,
    asset: &Address,
    account: &Account,
) {
    if account.is_isolated && !asset_config.isolation_borrow_enabled {
        panic_with_error!(env, EModeError::NotBorrowableIsolation);
    }

    if asset_config.is_siloed_borrowing && account.borrow_positions.len() > 1 {
        panic_with_error!(env, CollateralError::NotBorrowableSiloed);
    }

    // When any existing borrow or the new asset is siloed, every other
    // existing borrow must match the new asset.
    for existing in account.borrow_positions.keys() {
        if existing != *asset {
            let existing_config = cache.cached_asset_config(&existing);
            if existing_config.is_siloed_borrowing || asset_config.is_siloed_borrowing {
                panic_with_error!(env, CollateralError::NotBorrowableSiloed);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// process_borrow exactly
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn process_borrow(
    env: &Env,
    cache: &mut ControllerCache,
    account_id: u64,
    caller: &Address,
    account: &mut Account,
    asset: &Address,
    amount: i128,
    ltv_collateral: i128,
) {
    // validate_payment equivalent: asset must be supported, amount must be positive.
    validation::require_asset_supported(env, asset);
    validation::require_amount_positive(env, amount);

    let mut asset_config = cache.cached_asset_config(asset);
    let price_feed = cache.cached_price(asset);

    validate_borrow_asset(env, cache, &asset_config, asset, account);

    let asset_emode_config = emode::token_e_mode_config(env, account.e_mode_category_id, asset);
    emode::ensure_e_mode_compatible_with_asset(env, &asset_config, account.e_mode_category_id);
    let e_mode = emode::e_mode_category(env, account.e_mode_category_id);
    emode::apply_e_mode_to_asset_config(env, &mut asset_config, &e_mode, asset_emode_config);

    if !asset_config.is_borrowable {
        panic_with_error!(env, CollateralError::AssetNotBorrowable);
    }

    validate_ltv_collateral(
        env,
        cache,
        ltv_collateral,
        &account.borrow_positions,
        amount,
        &price_feed,
    );
    validate_borrow_cap(
        env,
        cache,
        &asset_config,
        amount,
        price_feed.asset_decimals,
        asset,
    );

    handle_isolated_debt(env, cache, account, amount, &price_feed);

    let borrow_position = get_or_create_borrow_position(account, account_id, &asset_config, asset);

    let updated_position = handle_borrow_position(
        env,
        cache,
        account_id,
        asset,
        amount,
        caller,
        &*account,
        borrow_position,
        &price_feed,
    );

    // Mutate the in-memory account so subsequent iterations of
    // `process_borrow` see the updated `borrow_positions` map.
    update::update_or_remove_position(account, &updated_position);
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use common::constants::RAY;
    use common::types::{
        MarketConfig, MarketParams, OraclePriceFluctuation, OracleProviderConfig, PoolKey,
        PoolState,
    };
    use soroban_sdk::testutils::{Address as _, Ledger, LedgerInfo};
    use soroban_sdk::{Address, Map, Symbol};

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
                protocol_version: 25,
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
            let pool = env.register(
                pool::LiquidityPool,
                (controller.clone(), params, controller.clone()),
            );

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

        fn market_config(&self, asset_config: AssetConfig) -> MarketConfig {
            MarketConfig {
                status: common::types::MarketStatus::Active,
                asset_config,
                pool_address: self.pool.clone(),
                oracle_config: OracleProviderConfig {
                    base_asset: self.asset.clone(),
                    oracle_type: common::types::OracleType::Normal,
                    exchange_source: common::types::ExchangeSource::SpotOnly,
                    asset_decimals: 7,
                    tolerance: OraclePriceFluctuation {
                        first_upper_ratio_bps: 10_200,
                        first_lower_ratio_bps: 9_800,
                        last_upper_ratio_bps: 11_000,
                        last_lower_ratio_bps: 9_000,
                    },
                    max_price_stale_seconds: 900,
                },
                cex_oracle: None,
                cex_asset_kind: common::types::ReflectorAssetKind::Stellar,
                cex_symbol: Symbol::new(&self.env, ""),
                cex_decimals: 0,
                dex_oracle: None,
                dex_asset_kind: common::types::ReflectorAssetKind::Stellar,
                dex_symbol: Symbol::new(&self.env, ""),
                dex_decimals: 0,
                twap_records: 0,
            }
        }

        fn account_with_two_borrows(&self) -> Account {
            let mut borrow_positions = Map::new(&self.env);
            for asset in [&self.asset, &self.other_asset] {
                borrow_positions.set(
                    asset.clone(),
                    AccountPosition {
                        position_type: common::types::AccountPositionType::Borrow,
                        asset: asset.clone(),
                        scaled_amount_ray: 1_0000000,
                        account_id: 1,
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
                mode: common::types::PositionMode::Normal,
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
                &t.market_config(t.asset_config(5_0000000, false)),
            );

            let mut cache = ControllerCache::new(&t.env, true);
            validate_borrow_cap(
                &t.env,
                &mut cache,
                &t.asset_config(5_0000000, false),
                1_0000000,
                7,
                &t.asset,
            );
        });
    }

    #[test]
    #[should_panic]
    fn test_validate_borrow_asset_rejects_siloed_asset_on_multi_borrow_account() {
        let t = TestSetup::new();

        t.as_controller(|| {
            let mut cache = ControllerCache::new(&t.env, true);
            let account = t.account_with_two_borrows();

            validate_borrow_asset(
                &t.env,
                &mut cache,
                &t.asset_config(0, true),
                &t.asset,
                &account,
            );
        });
    }
}
