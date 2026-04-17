use common::errors::{CollateralError, GenericError};
use common::events::{emit_update_position, UpdatePositionEvent};
use common::fp::Ray;
use common::types::{Account, AccountPosition, PoolPositionMutation};
use soroban_sdk::{panic_with_error, symbol_short, Address, Env, Vec};

use super::update;
use crate::cache::ControllerCache;
use crate::{storage, utils, validation};

pub fn process_repay(
    env: &Env,
    caller: &Address,
    account_id: u64,
    payments: &Vec<(Address, i128)>,
) {
    caller.require_auth();
    validation::require_not_paused(env);
    validation::require_not_flash_loaning(env);
    // Single storage read; no owner check because anyone can repay.
    let mut account = storage::get_account(env, account_id);

    let mut cache = ControllerCache::new_with_disabled_market_price(env, true);

    for (asset, amount) in payments {
        process_single_repay(env, caller, &mut account, &asset, amount, &mut cache);
    }

    // When the account closes out, remove it entirely instead of rewriting an empty account.
    if account.supply_positions.is_empty() && account.borrow_positions.is_empty() {
        utils::validate_account_is_empty(env, &account);
        utils::remove_account(env, account_id);
    } else {
        storage::set_account(env, account_id, &account);
    }

    // Flush the isolated-debt accumulator: one storage write and one event per
    // modified asset, regardless of how many repayments this batch made.
    cache.flush_isolated_debts();
}

fn process_single_repay(
    env: &Env,
    caller: &Address,
    account: &mut Account,
    asset: &Address,
    amount: i128,
    cache: &mut ControllerCache,
) {
    validation::require_amount_positive(env, amount);

    // The position must exist.
    let position = match account.borrow_positions.get(asset.clone()) {
        Some(pos) => pos,
        None => panic_with_error!(env, CollateralError::PositionNotFound),
    };

    // Transfer tokens from caller to pool using balance-delta accounting,
    // so fee-on-transfer or rebasing tokens cannot credit more debt repayment
    // than the pool actually received.
    let pool_addr = cache.cached_pool_address(asset);
    let token = soroban_sdk::token::Client::new(env, asset);
    let pool_balance_before = token.balance(&pool_addr);
    token.transfer(caller, &pool_addr, &amount);
    let pool_balance_after = token.balance(&pool_addr);
    let actual_received = pool_balance_after
        .checked_sub(pool_balance_before)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AmountMustBePositive));
    if actual_received <= 0 {
        panic_with_error!(env, GenericError::AmountMustBePositive);
    }

    // Shared repayment execution (also used by liquidation):
    // pool call, position update, isolated debt adjustment.
    let feed = cache.cached_price(asset);
    let result = execute_repayment(
        env,
        account,
        caller,
        &position,
        feed.price_wad,
        actual_received,
        cache,
    );

    // Repay uses borrow_index_ray.
    emit_update_position(
        env,
        UpdatePositionEvent {
            action: symbol_short!("repay"),
            index: result.market_index.borrow_index_ray,
            amount: result.actual_amount,
            position: result.position.clone().into(),
            asset_price: Some(feed.price_wad),
            caller: Some(caller.clone()),
            account_attributes: Some((&*account).into()),
        },
    );
}

// ---------------------------------------------------------------------------
// Shared repayment execution (also used by liquidation)
// ---------------------------------------------------------------------------

pub fn execute_repayment(
    env: &Env,
    account: &mut Account,
    caller: &Address,
    position: &AccountPosition,
    price_wad: i128,
    amount: i128,
    cache: &mut ControllerCache,
) -> PoolPositionMutation {
    let pool_addr = cache.cached_pool_address(&position.asset);
    let pool_client = pool_interface::LiquidityPoolClient::new(env, &pool_addr);

    let mut result = pool_client.repay(caller, &amount, position, &price_wad);

    // Derive the applied repayment from the pre-repay scaled debt and the
    // pool-returned synced borrow index. Prevents accounting drift when the
    // caller overpays: the applied amount is clamped to outstanding debt.
    let feed = cache.cached_price(&position.asset);
    let outstanding_before = Ray::from_raw(position.scaled_amount_ray)
        .mul(env, Ray::from_raw(result.market_index.borrow_index_ray))
        .to_asset(feed.asset_decimals);
    result.actual_amount = amount.min(outstanding_before);

    update::update_or_remove_position(account, &result.position);

    // Adjust isolated debt using the applied amount, not the requested amount.
    if account.is_isolated && result.actual_amount > 0 {
        let feed = cache.cached_price(&position.asset);
        utils::adjust_isolated_debt_usd(
            env,
            account,
            result.actual_amount,
            &price_wad,
            feed.asset_decimals,
            cache,
        );
    }

    result
}

pub fn clear_position_isolated_debt(
    env: &Env,
    position: &AccountPosition,
    account: &Account,
    cache: &mut ControllerCache,
) {
    if !account.is_isolated {
        return;
    }

    let market_index = cache.cached_market_index_readonly(&position.asset);
    let feed = cache.cached_price(&position.asset);
    let actual_amount = Ray::from_raw(position.scaled_amount_ray)
        .mul(env, Ray::from_raw(market_index.borrow_index_ray))
        .to_asset(feed.asset_decimals);

    if actual_amount > 0 {
        let feed = cache.cached_price(&position.asset);
        utils::adjust_isolated_debt_usd(
            env,
            account,
            actual_amount,
            &feed.price_wad,
            feed.asset_decimals,
            cache,
        );
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use common::constants::{RAY, WAD};
    use common::types::{
        AccountPositionType, AssetConfig, ExchangeSource, MarketConfig, MarketParams, MarketStatus,
        OraclePriceFluctuation, OracleProviderConfig, OracleType, PoolKey, PoolState, PositionMode,
        PriceFeed, ReflectorAssetKind, ReflectorConfig,
    };
    use soroban_sdk::testutils::{Address as _, Ledger, LedgerInfo};
    use soroban_sdk::{Address, Env, Map, Symbol};

    struct TestSetup {
        env: Env,
        controller: Address,
        owner: Address,
        asset: Address,
        pool: Address,
        reflector: Address,
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
            let owner = Address::generate(&env);
            let asset = env
                .register_stellar_asset_contract_v2(admin.clone())
                .address()
                .clone();
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

            let reflector = env.register(crate::helpers::testutils::TestReflector, ());
            let r_client = crate::helpers::testutils::TestReflectorClient::new(&env, &reflector);
            r_client.set_spot(
                &crate::helpers::testutils::TestReflectorAsset::Stellar(asset.clone()),
                &10_0000000_0000000i128,
                &1_000, // match ledger timestamp -- future-dated oracle prices now rejected
            );

            let setup = Self {
                env,
                controller,
                owner,
                asset,
                pool,
                reflector,
            };

            setup.as_controller(|| {
                crate::storage::set_market_config(&setup.env, &setup.asset, &setup.market_config());
                crate::storage::set_reflector_config(
                    &setup.env,
                    &setup.asset,
                    &ReflectorConfig {
                        cex_oracle: setup.reflector.clone(),
                        cex_asset_kind: ReflectorAssetKind::Stellar,
                        cex_symbol: soroban_sdk::Symbol::new(&setup.env, "USDC"),
                        cex_decimals: 14,
                        dex_oracle: None,
                        dex_asset_kind: ReflectorAssetKind::Stellar,
                        dex_decimals: 0,
                        twap_records: 0,
                    },
                );
            });

            setup
        }

        fn as_controller<T>(&self, f: impl FnOnce() -> T) -> T {
            self.env.as_contract(&self.controller, f)
        }

        fn market_config(&self) -> MarketConfig {
            MarketConfig {
                status: MarketStatus::Active,
                asset_config: AssetConfig {
                    loan_to_value_bps: 7_500,
                    liquidation_threshold_bps: 8_000,
                    liquidation_bonus_bps: 500,
                    liquidation_fees_bps: 100,
                    is_collateralizable: true,
                    is_borrowable: true,
                    e_mode_enabled: false,
                    is_isolated_asset: false,
                    is_siloed_borrowing: false,
                    is_flashloanable: true,
                    isolation_borrow_enabled: true,
                    isolation_debt_ceiling_usd_wad: 0,
                    flashloan_fee_bps: 9,
                    borrow_cap: i128::MAX,
                    supply_cap: i128::MAX,
                },
                pool_address: self.pool.clone(),
                oracle_config: OracleProviderConfig {
                    base_asset: self.asset.clone(),
                    oracle_type: OracleType::Normal,
                    exchange_source: ExchangeSource::SpotOnly,
                    asset_decimals: 7,
                    tolerance: OraclePriceFluctuation {
                        first_upper_ratio_bps: 10_200,
                        first_lower_ratio_bps: 9_800,
                        last_upper_ratio_bps: 11_000,
                        last_lower_ratio_bps: 9_000,
                    },
                    max_price_stale_seconds: 900,
                },
                cex_oracle: Some(self.reflector.clone()),
                cex_asset_kind: ReflectorAssetKind::Stellar,
                cex_symbol: Symbol::new(&self.env, "USDC"),
                cex_decimals: 14,
                dex_oracle: None,
                dex_asset_kind: ReflectorAssetKind::Stellar,
                dex_symbol: Symbol::new(&self.env, ""),
                dex_decimals: 0,
                twap_records: 0,
            }
        }

        fn borrow_position(&self, account_id: u64, scaled_amount_ray: i128) -> AccountPosition {
            AccountPosition {
                position_type: AccountPositionType::Borrow,
                asset: self.asset.clone(),
                scaled_amount_ray,
                account_id,
                liquidation_threshold_bps: 8_000,
                liquidation_bonus_bps: 500,
                liquidation_fees_bps: 100,
                loan_to_value_bps: 7_500,
            }
        }

        fn account_with_borrow_only(&self, account_id: u64, is_isolated: bool) -> Account {
            let mut borrow_positions = Map::new(&self.env);
            borrow_positions.set(
                self.asset.clone(),
                self.borrow_position(account_id, RAY), // 1 token in RAY-native
            );

            Account {
                owner: self.owner.clone(),
                is_isolated,
                e_mode_category_id: 0,
                mode: PositionMode::Normal,
                isolated_asset: is_isolated.then(|| self.asset.clone()),
                supply_positions: Map::new(&self.env),
                borrow_positions,
            }
        }
    }

    #[test]
    fn test_process_repay_removes_account_when_last_borrow_is_closed() {
        let t = TestSetup::new();
        let account_id = 1;
        let repay_amount = 1_0000000i128;

        soroban_sdk::token::StellarAssetClient::new(&t.env, &t.asset).mint(&t.owner, &repay_amount);

        t.as_controller(|| {
            // M-03: constructor pauses the contract. Unpause for this test
            // since it bypasses the client and calls process_repay directly,
            // which checks `require_not_paused`.
            stellar_contract_utils::pausable::unpause(&t.env);
            storage::set_market_config(&t.env, &t.asset, &t.market_config());
            storage::set_account(
                &t.env,
                account_id,
                &t.account_with_borrow_only(account_id, false),
            );

            t.env.as_contract(&t.pool, || {
                t.env.storage().instance().set(
                    &PoolKey::State,
                    &PoolState {
                        supplied_ray: 0,
                        borrowed_ray: RAY, // 1 token scaled to RAY (matches position)
                        revenue_ray: 0,
                        borrow_index_ray: RAY,
                        supply_index_ray: RAY,
                        last_timestamp: t.env.ledger().timestamp() * 1000,
                    },
                );
            });

            let payments = soroban_sdk::vec![&t.env, (t.asset.clone(), repay_amount)];
            process_repay(&t.env, &t.owner, account_id, &payments);

            assert!(
                storage::try_get_account(&t.env, account_id).is_none(),
                "fully repaid debt-only accounts should be removed"
            );
        });
    }

    #[test]
    fn test_clear_position_isolated_debt_updates_cache_for_isolated_accounts() {
        let t = TestSetup::new();
        let account = t.account_with_borrow_only(1, true);
        let position = t.borrow_position(1, RAY); // 1 token in RAY-native

        t.as_controller(|| {
            let market = t.market_config();
            storage::set_market_config(&t.env, &t.asset, &market);
            storage::set_reflector_config(
                &t.env,
                &t.asset,
                &ReflectorConfig {
                    cex_oracle: t.reflector.clone(),
                    cex_asset_kind: ReflectorAssetKind::Stellar,
                    cex_symbol: soroban_sdk::Symbol::new(&t.env, "USDC"),
                    cex_decimals: 14,
                    dex_oracle: None,
                    dex_asset_kind: ReflectorAssetKind::Stellar,
                    dex_decimals: 0,
                    twap_records: 0,
                },
            );
            storage::set_oracle_config(&t.env, &t.asset, &market.oracle_config);
            t.env.as_contract(&t.pool, || {
                t.env.storage().instance().set(
                    &PoolKey::State,
                    &PoolState {
                        supplied_ray: 0,
                        borrowed_ray: RAY, // 1 token in RAY-native
                        revenue_ray: 0,
                        borrow_index_ray: RAY,
                        supply_index_ray: RAY,
                        last_timestamp: t.env.ledger().timestamp() * 1000,
                    },
                );
            });

            let mut cache = ControllerCache::new(&t.env, true);
            cache.set_price(
                &t.asset,
                &PriceFeed {
                    price_wad: WAD,
                    asset_decimals: 7,
                    timestamp: 1_000,
                },
            );
            cache.set_isolated_debt(&t.asset, 2 * WAD);

            clear_position_isolated_debt(&t.env, &position, &account, &mut cache);

            // 2 WAD - (1 token × $1) = 1 WAD.
            assert_eq!(cache.get_isolated_debt(&t.asset), WAD);
        });
    }
}
