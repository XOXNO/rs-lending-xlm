use common::errors::{CollateralError, GenericError};
use common::events::{emit_update_position, UpdatePositionEvent};
use common::fp::Ray;
use common::types::{Account, AccountPosition, Payment, PoolPositionMutation};
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
    let mut account =
        storage::account_from_parts(meta, Map::new(env), borrow_positions);
    let mut cache = ControllerCache::new_with_disabled_market_price(env, allow_unsafe);

    let repayment_plan = utils::aggregate_positive_payments(env, payments);
    for (asset, amount) in repayment_plan {
        process_single_repay(env, caller, &mut account, &asset, amount, &mut cache);
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
pub fn execute_repayment(
    env: &Env,
    account: &mut Account,
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

    let mut result = pool_repay_call(env, caller.clone(), amount, position.clone(), price_wad);

    let feed = cache.cached_price(&position.asset);
    let outstanding_before = actual_borrow_amount(
        env,
        position,
        result.market_index.borrow_index_ray,
        feed.asset_decimals,
    );
    result.actual_amount = amount.min(outstanding_before);

    update::update_or_remove_position(account, &result.position);
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
            position: result.position.clone().into(),
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
    position: &AccountPosition,
    account: &Account,
    cache: &mut ControllerCache,
) {
    if !account.is_isolated {
        return;
    }

    let market_index = cache.cached_market_index(&position.asset);
    let feed = cache.cached_price(&position.asset);
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
    crate::spec::summaries::pool::repay_summary,
    fn pool_repay_call(
        env: &Env,
        caller: Address,
        amount: i128,
        position: AccountPosition,
        price_wad: i128,
    ) -> PoolPositionMutation {
        let pool_addr = crate::storage::get_market_config(env, &position.asset).pool_address;
        pool_interface::LiquidityPoolClient::new(env, &pool_addr).repay(
            &caller,
            &amount,
            &position,
            &price_wad,
        )
    }
);

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
            let pool = env.register(pool::LiquidityPool, (controller.clone(), params));

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
    #[should_panic(expected = "Error(Contract, #16)")]
    fn test_repay_rejects_empty_payments() {
        let t = TestSetup::new();

        t.as_controller(|| {
            let payments = soroban_sdk::Vec::new(&t.env);
            process_repay(&t.env, &t.owner, 1, &payments);
        });
    }

    #[test]
    fn test_repay_does_not_delete_account_when_empty() {
        let t = TestSetup::new();
        let account_id = 1;
        let repay_amount = 1_0000000i128;

        soroban_sdk::token::StellarAssetClient::new(&t.env, &t.asset).mint(&t.owner, &repay_amount);

        t.as_controller(|| {
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

            let account = storage::try_get_account(&t.env, account_id)
                .expect("account must remain after full repay (only withdraw burns)");
            assert!(
                account.borrow_positions.is_empty(),
                "borrow position should be cleared after full repay"
            );
            assert!(
                account.supply_positions.is_empty(),
                "supply map remains empty as before"
            );
            assert_eq!(account.owner, t.owner);
        });
    }

    #[test]
    fn test_repay_works_when_oracle_is_stale() {
        let t = TestSetup::new();
        let account_id = 2;
        let repay_amount = 1_0000000i128;

        soroban_sdk::token::StellarAssetClient::new(&t.env, &t.asset).mint(&t.owner, &repay_amount);

        t.as_controller(|| {
            stellar_contract_utils::pausable::unpause(&t.env);
            storage::set_market_config(&t.env, &t.asset, &t.market_config());
            storage::set_account(
                &t.env,
                account_id,
                &t.account_with_borrow_only(account_id, false),
            );

            let now = t.env.ledger().timestamp();
            t.env.ledger().set(LedgerInfo {
                timestamp: now + 1_500,
                protocol_version: 26,
                sequence_number: 100,
                network_id: Default::default(),
                base_reserve: 10,
                min_temp_entry_ttl: 10,
                min_persistent_entry_ttl: 10,
                max_entry_ttl: 3_110_400,
            });

            t.env.as_contract(&t.pool, || {
                t.env.storage().instance().set(
                    &PoolKey::State,
                    &PoolState {
                        supplied_ray: 0,
                        borrowed_ray: RAY,
                        revenue_ray: 0,
                        borrow_index_ray: RAY,
                        supply_index_ray: RAY,
                        last_timestamp: t.env.ledger().timestamp() * 1000,
                    },
                );
            });

            let payments = soroban_sdk::vec![&t.env, (t.asset.clone(), repay_amount)];
            process_repay(&t.env, &t.owner, account_id, &payments);

            let account = storage::try_get_account(&t.env, account_id)
                .expect("account must persist after stale-oracle repay");
            assert!(
                account.borrow_positions.is_empty(),
                "stale-oracle repay must still clear the debt"
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
