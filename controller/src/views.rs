use common::types::{
    AccountAttributes, AccountMeta, AssetExtendedConfigView, EModeCategory, LiquidationEstimate,
    MarketConfig, MarketIndexView, PaymentTuple, POSITION_TYPE_BORROW, POSITION_TYPE_DEPOSIT,
};
use common::{
    constants::WAD,
    fp::{Ray, Wad},
};
use soroban_sdk::{Address, Env, Vec};

use crate::cache::ControllerCache;
use crate::{helpers, storage};

fn try_get_account_meta(env: &Env, account_id: u64) -> Option<AccountMeta> {
    storage::try_get_account_meta(env, account_id)
}

pub fn health_factor(env: &Env, account_id: u64) -> i128 {
    let mut cache = ControllerCache::new_view(env);
    match storage::try_get_account(env, account_id) {
        Some(account) => helpers::calculate_health_factor(
            env,
            &mut cache,
            &account.supply_positions,
            &account.borrow_positions,
        ),
        None => i128::MAX,
    }
}

pub fn can_be_liquidated(env: &Env, account_id: u64) -> bool {
    health_factor(env, account_id) < WAD
}

pub fn total_collateral_in_usd(env: &Env, account_id: u64) -> i128 {
    let meta = match try_get_account_meta(env, account_id) {
        Some(meta) => meta,
        None => return 0,
    };
    if meta.supply_assets.is_empty() {
        return 0;
    }

    let mut cache = ControllerCache::new_view(env);
    let mut total_collateral = Wad::ZERO;

    for asset in meta.supply_assets.iter() {
        let position =
            storage::get_account_position(env, account_id, POSITION_TYPE_DEPOSIT, &asset);
        let feed = cache.cached_price(&position.asset);
        let market_index = cache.cached_market_index(&position.asset);

        let value = helpers::position_value(
            env,
            Ray::from_raw(position.scaled_amount_ray),
            Ray::from_raw(market_index.supply_index_ray),
            Wad::from_raw(feed.price_wad),
        );
        total_collateral = total_collateral + value;
    }

    total_collateral.raw()
}

pub fn total_borrow_in_usd(env: &Env, account_id: u64) -> i128 {
    let meta = match try_get_account_meta(env, account_id) {
        Some(meta) => meta,
        None => return 0,
    };
    if meta.borrow_assets.is_empty() {
        return 0;
    }

    let mut cache = ControllerCache::new_view(env);
    let mut total_borrow = Wad::ZERO;

    for asset in meta.borrow_assets.iter() {
        let position = storage::get_account_position(env, account_id, POSITION_TYPE_BORROW, &asset);
        let feed = cache.cached_price(&position.asset);
        let market_index = cache.cached_market_index(&position.asset);

        let value = helpers::position_value(
            env,
            Ray::from_raw(position.scaled_amount_ray),
            Ray::from_raw(market_index.borrow_index_ray),
            Wad::from_raw(feed.price_wad),
        );
        total_borrow = total_borrow + value;
    }

    total_borrow.raw()
}

pub fn collateral_amount_for_token(env: &Env, account_id: u64, asset: &Address) -> i128 {
    let position =
        match storage::try_get_account_position(env, account_id, POSITION_TYPE_DEPOSIT, asset) {
            Some(position) => position,
            None => return 0,
        };

    let mut cache = ControllerCache::new_view(env);
    let market_index = cache.cached_market_index(asset);
    let feed = cache.cached_price(asset);

    Ray::from_raw(position.scaled_amount_ray)
        .mul(env, Ray::from_raw(market_index.supply_index_ray))
        .to_asset(feed.asset_decimals)
}

pub fn borrow_amount_for_token(env: &Env, account_id: u64, asset: &Address) -> i128 {
    let position =
        match storage::try_get_account_position(env, account_id, POSITION_TYPE_BORROW, asset) {
            Some(position) => position,
            None => return 0,
        };

    let mut cache = ControllerCache::new_view(env);
    let market_index = cache.cached_market_index(asset);
    let feed = cache.cached_price(asset);

    Ray::from_raw(position.scaled_amount_ray)
        .mul(env, Ray::from_raw(market_index.borrow_index_ray))
        .to_asset(feed.asset_decimals)
}

pub fn get_account_positions(
    env: &Env,
    account_id: u64,
) -> (
    Vec<common::types::AccountPosition>,
    Vec<common::types::AccountPosition>,
) {
    let meta = match try_get_account_meta(env, account_id) {
        Some(meta) => meta,
        None => return (Vec::new(env), Vec::new(env)),
    };

    let mut supply = Vec::new(env);
    for asset in meta.supply_assets.iter() {
        if let Some(position) =
            storage::try_get_account_position(env, account_id, POSITION_TYPE_DEPOSIT, &asset)
        {
            supply.push_back(position);
        }
    }

    let mut borrow = Vec::new(env);
    for asset in meta.borrow_assets.iter() {
        if let Some(position) =
            storage::try_get_account_position(env, account_id, POSITION_TYPE_BORROW, &asset)
        {
            borrow.push_back(position);
        }
    }

    (supply, borrow)
}

pub fn get_account_attributes(env: &Env, account_id: u64) -> AccountAttributes {
    let meta = storage::get_account_meta(env, account_id);
    AccountAttributes::from(&meta)
}

pub fn get_market_config_view(env: &Env, asset: &Address) -> MarketConfig {
    storage::get_market_config(env, asset)
}

pub fn get_emode_category_view(env: &Env, category_id: u32) -> EModeCategory {
    storage::get_emode_category(env, category_id)
}

pub fn get_isolated_debt_view(env: &Env, asset: &Address) -> i128 {
    storage::get_isolated_debt(env, asset)
}

pub fn liquidation_collateral_available(env: &Env, account_id: u64) -> i128 {
    let account = match storage::try_get_account(env, account_id) {
        Some(account) => account,
        None => return 0,
    };
    let mut cache = ControllerCache::new_view(env);
    let (_, _, weighted_coll) = helpers::calculate_account_totals(
        env,
        &mut cache,
        &account.supply_positions,
        &account.borrow_positions,
    );
    weighted_coll.raw()
}

pub fn ltv_collateral_in_usd(env: &Env, account_id: u64) -> i128 {
    let account = match storage::try_get_account(env, account_id) {
        Some(account) => account,
        None => return 0,
    };
    let mut cache = ControllerCache::new_view(env);
    helpers::calculate_ltv_collateral_wad(env, &mut cache, &account.supply_positions).raw()
}

// ---------------------------------------------------------------------------
// Market index views
// ---------------------------------------------------------------------------

pub fn get_all_markets_detailed(env: &Env, assets: &Vec<Address>) -> Vec<AssetExtendedConfigView> {
    let mut cache = ControllerCache::new_view(env);
    let mut result = Vec::new(env);

    for i in 0..assets.len() {
        let asset = assets.get(i).unwrap();
        let market = cache.cached_market_config(&asset);
        let final_price = crate::oracle::token_price(&mut cache, &asset).price_wad;
        result.push_back(AssetExtendedConfigView {
            asset,
            pool_address: market.pool_address,
            price_wad: final_price,
        });
    }

    result
}

pub fn get_all_market_indexes_detailed(env: &Env, assets: &Vec<Address>) -> Vec<MarketIndexView> {
    let mut cache = ControllerCache::new_view(env);
    let mut result = Vec::new(env);

    for i in 0..assets.len() {
        let asset = assets.get(i).unwrap();
        let index = cache.cached_market_index(&asset);
        let (aggregator_price, safe_price, final_price, within_first, within_second) =
            crate::oracle::price_components(&mut cache, &asset);
        let safe_price_wad = safe_price.unwrap_or(final_price);
        let aggregator_price_wad = aggregator_price.unwrap_or(final_price);

        result.push_back(MarketIndexView {
            asset,
            supply_index_ray: index.supply_index_ray,
            borrow_index_ray: index.borrow_index_ray,
            price_wad: final_price,
            safe_price_wad,
            aggregator_price_wad,
            within_first_tolerance: within_first,
            within_second_tolerance: within_second,
        });
    }

    result
}

// ---------------------------------------------------------------------------
// Liquidation estimation view
// ---------------------------------------------------------------------------

pub fn liquidation_estimations_detailed(
    env: &Env,
    account_id: u64,
    debt_payments: &Vec<(Address, i128)>,
) -> LiquidationEstimate {
    let mut cache = ControllerCache::new_view(env);
    let account = storage::get_account(env, account_id);
    let (seized, _repaid, refunds, max_payment_wad, bonus_rate_bps) =
        crate::positions::liquidation::execute_liquidation(
            env,
            &account,
            debt_payments,
            &mut cache,
        );

    let mut seized_collaterals = Vec::new(env);
    let mut protocol_fees = Vec::new(env);
    for i in 0..seized.len() {
        let (asset, amount, protocol_fee, _feed, _index) = seized.get(i).unwrap();
        seized_collaterals.push_back(PaymentTuple {
            asset: asset.clone(),
            amount,
        });
        protocol_fees.push_back(PaymentTuple {
            asset,
            amount: protocol_fee,
        });
    }

    let mut refunds_view = Vec::new(env);
    for i in 0..refunds.len() {
        let (asset, amount) = refunds.get(i).unwrap();
        refunds_view.push_back(PaymentTuple { asset, amount });
    }

    LiquidationEstimate {
        seized_collaterals,
        protocol_fees,
        refunds: refunds_view,
        max_payment_wad,
        bonus_rate_bps,
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::ControllerClient;
    use common::constants::RAY;
    use common::types::{
        Account, AccountPosition, AssetConfig, MarketConfig, MarketParams, MarketStatus,
        OraclePriceFluctuation, OracleProviderConfig, PoolState, ReflectorAssetKind,
        ReflectorConfig,
    };
    use soroban_sdk::testutils::{Address as _, Ledger, LedgerInfo};
    use soroban_sdk::{contract, contractimpl, contracttype, Address, Map, Symbol, Vec};

    #[contracttype]
    #[derive(Clone)]
    enum TestReflectorAsset {
        Stellar(Address),
        Other(Symbol),
    }

    #[contracttype]
    #[derive(Clone)]
    struct TestReflectorPriceData {
        price: i128,
        timestamp: u64,
    }

    #[contract]
    struct TestReflector;

    #[contractimpl]
    impl TestReflector {
        pub fn set_spot(env: Env, asset: TestReflectorAsset, price: i128, timestamp: u64) {
            env.storage()
                .temporary()
                .set(&asset, &TestReflectorPriceData { price, timestamp });
        }

        pub fn decimals(_env: Env) -> u32 {
            14
        }

        pub fn resolution(_env: Env) -> u32 {
            300
        }

        pub fn lastprice(env: Env, asset: TestReflectorAsset) -> Option<TestReflectorPriceData> {
            env.storage().temporary().get(&asset)
        }

        pub fn prices(
            env: Env,
            asset: TestReflectorAsset,
            records: u32,
        ) -> Vec<Option<TestReflectorPriceData>> {
            let mut out = Vec::new(&env);
            let spot = Self::lastprice(env.clone(), asset);
            for _ in 0..records {
                out.push_back(spot.clone());
            }
            out
        }
    }

    struct TestSetup {
        env: Env,
        controller: Address,
        asset_a: Address,
        asset_b: Address,
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
            let asset_a = env
                .register_stellar_asset_contract_v2(admin.clone())
                .address()
                .clone();
            let asset_b = env
                .register_stellar_asset_contract_v2(admin)
                .address()
                .clone();

            Self {
                env,
                controller,
                asset_a,
                asset_b,
            }
        }

        fn as_controller<T>(&self, f: impl FnOnce() -> T) -> T {
            self.env.as_contract(&self.controller, f)
        }

        fn market_config(&self, asset: &Address) -> MarketConfig {
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
            let pool = self.env.register(
                pool::LiquidityPool,
                (self.controller.clone(), params, self.controller.clone()),
            );
            self.env.as_contract(&pool, || {
                self.env.storage().instance().set(
                    &common::types::PoolKey::State,
                    &PoolState {
                        supplied_ray: 0,
                        borrowed_ray: 0,
                        revenue_ray: 0,
                        borrow_index_ray: RAY,
                        supply_index_ray: RAY,
                        last_timestamp: self.env.ledger().timestamp() * 1000,
                    },
                );
            });

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
                    isolation_debt_ceiling_usd_wad: 1_000_000,
                    flashloan_fee_bps: 9,
                    borrow_cap: 0,
                    supply_cap: 0,
                },
                pool_address: pool,
                oracle_config: OracleProviderConfig {
                    base_asset: asset.clone(),
                    oracle_type: common::types::OracleType::None,
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

        fn empty_account(&self, owner: Address) -> Account {
            Account {
                owner,
                is_isolated: false,
                e_mode_category_id: 0,
                mode: common::types::PositionMode::Normal,
                isolated_asset: None,
                supply_positions: Map::new(&self.env),
                borrow_positions: Map::new(&self.env),
            }
        }

        fn configure_two_markets(&self) {
            self.as_controller(|| {
                let market_a = self.market_config(&self.asset_a);
                let market_b = self.market_config(&self.asset_b);
                storage::set_market_config(&self.env, &self.asset_a, &market_a);
                storage::set_market_config(&self.env, &self.asset_b, &market_b);
                storage::add_to_pools_list(&self.env, &self.asset_a, &market_a.pool_address);
                storage::add_to_pools_list(&self.env, &self.asset_b, &market_b.pool_address);
            });
        }
    }

    #[test]
    fn test_missing_account_views_return_safe_defaults() {
        let t = TestSetup::new();
        t.configure_two_markets();

        t.as_controller(|| {
            assert_eq!(health_factor(&t.env, 999), i128::MAX);
            assert_eq!(total_collateral_in_usd(&t.env, 999), 0);
            assert_eq!(total_borrow_in_usd(&t.env, 999), 0);
            assert_eq!(collateral_amount_for_token(&t.env, 999, &t.asset_a), 0);
            assert_eq!(borrow_amount_for_token(&t.env, 999, &t.asset_a), 0);
            assert_eq!(health_factor(&t.env, 999), i128::MAX);
        });
    }

    #[test]
    fn test_empty_account_views_return_zero_balances() {
        let t = TestSetup::new();
        t.configure_two_markets();

        t.as_controller(|| {
            let owner = Address::generate(&t.env);
            storage::set_account(&t.env, 1, &t.empty_account(owner));

            assert_eq!(total_collateral_in_usd(&t.env, 1), 0);
            assert_eq!(total_borrow_in_usd(&t.env, 1), 0);
            assert_eq!(collateral_amount_for_token(&t.env, 1, &t.asset_a), 0);
            assert_eq!(borrow_amount_for_token(&t.env, 1, &t.asset_a), 0);
        });
    }

    #[test]
    fn test_get_all_market_indexes_through_contract_wrapper() {
        let t = TestSetup::new();
        t.configure_two_markets();

        let oracle = t.env.register(TestReflector, ());
        let oracle_client = TestReflectorClient::new(&t.env, &oracle);
        oracle_client.set_spot(
            &TestReflectorAsset::Stellar(t.asset_a.clone()),
            &100_000_000_000_000,
            &1_000,
        );
        oracle_client.set_spot(
            &TestReflectorAsset::Stellar(t.asset_b.clone()),
            &200_000_000_000_000,
            &1_000,
        );

        t.as_controller(|| {
            for asset in [t.asset_a.clone(), t.asset_b.clone()] {
                let mut market = storage::get_market_config(&t.env, &asset);
                market.oracle_config.oracle_type = common::types::OracleType::Normal;
                market.oracle_config.exchange_source = common::types::ExchangeSource::SpotOnly;
                storage::set_market_config(&t.env, &asset, &market);
                storage::set_reflector_config(
                    &t.env,
                    &asset,
                    &ReflectorConfig {
                        cex_oracle: oracle.clone(),
                        cex_asset_kind: ReflectorAssetKind::Stellar,
                        cex_symbol: Symbol::new(&t.env, "XLM"),
                        cex_decimals: 14,
                        dex_oracle: None,
                        dex_asset_kind: ReflectorAssetKind::Stellar,
                        dex_decimals: 0,
                        twap_records: 0,
                    },
                );
            }
        });

        let client = ControllerClient::new(&t.env, &t.controller);
        let assets = Vec::from_array(&t.env, [t.asset_a.clone(), t.asset_b.clone()]);
        let indexes = client.get_all_market_indexes_detailed(&assets);

        assert_eq!(indexes.len(), 2);
        assert_eq!(indexes.get(0).unwrap().borrow_index_ray, RAY);
        assert_eq!(indexes.get(1).unwrap().supply_index_ray, RAY);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #13)")]
    fn test_get_account_attributes_panics_for_missing_account() {
        let t = TestSetup::new();
        t.as_controller(|| {
            let _ = get_account_attributes(&t.env, 404);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #13)")]
    fn test_get_account_owner_panics_for_missing_account() {
        let t = TestSetup::new();
        t.as_controller(|| {
            let _ = storage::get_account_meta(&t.env, 404).owner;
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #101)")]
    fn test_liquidation_estimations_panics_when_account_is_healthy() {
        let t = TestSetup::new();
        t.configure_two_markets();

        t.as_controller(|| {
            let owner = Address::generate(&t.env);
            storage::set_account(&t.env, 2, &t.empty_account(owner));
            let debt_payments = Vec::new(&t.env);
            let _ = liquidation_estimations_detailed(&t.env, 2, &debt_payments);
        });
    }

    #[test]
    fn test_liquidation_estimations_handles_zero_total_collateral() {
        let t = TestSetup::new();
        let oracle = t.env.register(TestReflector, ());
        let oracle_client = TestReflectorClient::new(&t.env, &oracle);
        let account_id = 7u64;

        oracle_client.set_spot(
            &TestReflectorAsset::Stellar(t.asset_a.clone()),
            &200_000_000_000_000,
            &1_000,
        );

        t.as_controller(|| {
            let mut market = t.market_config(&t.asset_a);
            market.oracle_config.oracle_type = common::types::OracleType::Normal;
            market.oracle_config.exchange_source = common::types::ExchangeSource::SpotOnly;
            storage::set_market_config(&t.env, &t.asset_a, &market);
            storage::set_reflector_config(
                &t.env,
                &t.asset_a,
                &ReflectorConfig {
                    cex_oracle: oracle.clone(),
                    cex_asset_kind: ReflectorAssetKind::Stellar,
                    cex_symbol: Symbol::new(&t.env, "XLM"),
                    cex_decimals: 14,
                    dex_oracle: None,
                    dex_asset_kind: ReflectorAssetKind::Stellar,
                    dex_decimals: 0,
                    twap_records: 0,
                },
            );

            let mut borrow_positions = Map::new(&t.env);
            borrow_positions.set(
                t.asset_a.clone(),
                AccountPosition {
                    position_type: common::types::AccountPositionType::Borrow,
                    asset: t.asset_a.clone(),
                    scaled_amount_ray: 5 * RAY, // 5 tokens in RAY-native
                    account_id,
                    liquidation_threshold_bps: 8_000,
                    liquidation_bonus_bps: 500,
                    liquidation_fees_bps: 100,
                    loan_to_value_bps: 7_500,
                },
            );
            storage::set_account(
                &t.env,
                account_id,
                &Account {
                    owner: Address::generate(&t.env),
                    is_isolated: false,
                    e_mode_category_id: 0,
                    mode: common::types::PositionMode::Normal,
                    isolated_asset: None,
                    supply_positions: Map::new(&t.env),
                    borrow_positions,
                },
            );
        });

        t.as_controller(|| {
            let debt_payments = Vec::from_array(&t.env, [(t.asset_a.clone(), 5_0000000)]);
            let estimate = liquidation_estimations_detailed(&t.env, account_id, &debt_payments);

            assert_eq!(estimate.seized_collaterals.len(), 0);
            assert_eq!(estimate.protocol_fees.len(), 0);
            assert_eq!(estimate.max_payment_wad, 0);
            assert_eq!(health_factor(&t.env, account_id), 0);
        });
    }

    #[test]
    fn test_local_test_reflector_exposes_spot_and_history_helpers() {
        let t = TestSetup::new();
        let oracle = t.env.register(TestReflector, ());
        let oracle_client = TestReflectorClient::new(&t.env, &oracle);
        let stellar_asset = TestReflectorAsset::Stellar(t.asset_a.clone());
        let other_asset = TestReflectorAsset::Other(Symbol::new(&t.env, "BTC"));

        oracle_client.set_spot(&stellar_asset, &123, &456);
        oracle_client.set_spot(&other_asset, &321, &654);

        assert_eq!(oracle_client.decimals(), 14);
        assert_eq!(oracle_client.resolution(), 300);
        assert_eq!(
            oracle_client.lastprice(&stellar_asset).unwrap().timestamp,
            456
        );
        assert_eq!(oracle_client.lastprice(&other_asset).unwrap().price, 321);

        let history = oracle_client.prices(&stellar_asset, &2);
        assert_eq!(history.len(), 2);
        assert_eq!(history.get(0).unwrap().unwrap().price, 123);
        assert_eq!(history.get(1).unwrap().unwrap().timestamp, 456);
    }
}
