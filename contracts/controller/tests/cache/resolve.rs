use super::*;
use crate::Controller;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

fn spoke_asset() -> SpokeAssetConfig {
    SpokeAssetConfig {
        is_collateralizable: true,
        is_borrowable: true,
        paused: false,
        frozen: false,
        loan_to_value: 9_000,
        liquidation_threshold: 9_300,
        liquidation_bonus: 300,
        liquidation_fees: 0,
        supply_cap: 0,
        borrow_cap: 0,
    }
}

// Token-rooted: returns the stored `AssetOracle`, independent of spoke.
#[test]
fn resolve_default_returns_asset_oracle_base() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    let asset = Address::generate(&env);

    env.as_contract(&contract_id, || {
        let base = MarketOracleConfig::pending_for(asset.clone(), 7);
        storage::set_asset_oracle(&env, &asset, &base);

        assert_eq!(storage::get_asset_oracle(&env, &asset), Some(base.clone()));

        let mut cache = Cache::new_view(&env);
        assert_eq!(cache.cached_asset_oracle(&asset), base);
    });
}

// Spoke listing does not divert resolution from the token `AssetOracle` base.
#[test]
fn resolve_spoke_without_override_falls_back_to_base() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    let asset = Address::generate(&env);
    let spoke_id = 1u32;

    env.as_contract(&contract_id, || {
        let base = MarketOracleConfig::pending_for(asset.clone(), 7);
        storage::set_asset_oracle(&env, &asset, &base);

        let hub_asset = HubAssetKey {
            hub_id: 0,
            asset: asset.clone(),
        };
        storage::set_spoke_asset(
            &env,
            spoke_id,
            &hub_asset,
            &spoke_asset(),
        );

        let mut cache = Cache::new_view(&env);
        cache.ensure_spoke_context(spoke_id);
        assert_eq!(cache.cached_asset_oracle(&asset), base);
    });
}

// Re-entry of an in-flight asset reverts `OracleCycleDetected` (#225).
#[test]
#[should_panic(expected = "Error(Contract, #225)")]
fn price_resolution_reentry_reverts_cycle() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    let asset = Address::generate(&env);

    env.as_contract(&contract_id, || {
        let mut cache = Cache::new_view(&env);
        cache.enter_price_resolution(&asset);
        // Re-entry mid-resolution is a cycle.
        cache.enter_price_resolution(&asset);
    });
}

// Distinct assets nest freely; sequential re-resolve after pop is not a cycle.
#[test]
fn price_resolution_allows_distinct_and_sequential() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    let a = Address::generate(&env);
    let b = Address::generate(&env);

    env.as_contract(&contract_id, || {
        let mut cache = Cache::new_view(&env);
        cache.enter_price_resolution(&a);
        cache.enter_price_resolution(&b);
        cache.exit_price_resolution();
        cache.exit_price_resolution();
        cache.enter_price_resolution(&a);
        cache.exit_price_resolution();
    });
}

// Mutual Quoted anchors: `token_price` hits the cycle guard (#225).
#[test]
#[should_panic(expected = "Error(Contract, #225)")]
fn token_price_mutual_quote_cycle_reverts() {
    use common::types::{
        OracleAssetRef, OraclePriceFluctuation, OracleReadMode, OracleSourceConfig,
        OracleSourceConfigOption, OracleStrategy, ReflectorBase, ReflectorSourceConfig,
    };
    use mock_oracle::{
        MockReflectorOracle, MockReflectorOracleClient, ReflectorAsset as MockAsset,
    };

    let env = Env::default();
    let admin = Address::generate(&env);
    let controller_id = env.register(Controller, (admin,));
    let oracle_id = env.register(MockReflectorOracle, ());
    let a = Address::generate(&env);
    let b = Address::generate(&env);

    let mock = MockReflectorOracleClient::new(&env, &oracle_id);
    let one_usd_wad: i128 = 1_000_000_000_000_000_000;
    mock.set_price(&MockAsset::Stellar(a.clone()), &one_usd_wad);
    mock.set_price(&MockAsset::Stellar(b.clone()), &one_usd_wad);

    // USD primary + anchor quoted in `quote` (mutual pair → cycle).
    let cfg = |asset: &Address, quote: &Address| MarketOracleConfig {
        asset_decimals: 7,
        max_price_stale_seconds: 900,
        tolerance: OraclePriceFluctuation {
            upper_ratio_bps: 10_500,
            lower_ratio_bps: 9_500,
        },
        strategy: OracleStrategy::PrimaryWithAnchor,
        primary: OracleSourceConfig::Reflector(ReflectorSourceConfig {
            contract: oracle_id.clone(),
            asset: OracleAssetRef::Stellar(asset.clone()),
            read_mode: OracleReadMode::Spot,
            decimals: 14,
            resolution_seconds: 300,
            base: ReflectorBase::Usd,
        }),
        anchor: OracleSourceConfigOption::Some(OracleSourceConfig::Reflector(
            ReflectorSourceConfig {
                contract: oracle_id.clone(),
                asset: OracleAssetRef::Stellar(asset.clone()),
                read_mode: OracleReadMode::Spot,
                decimals: 14,
                resolution_seconds: 300,
                base: ReflectorBase::Quoted(quote.clone()),
            },
        )),
        min_sanity_price_wad: 0,
        max_sanity_price_wad: i128::MAX,
    };

    env.as_contract(&controller_id, || {
        storage::set_asset_oracle(&env, &a, &cfg(&a, &b));
        storage::set_asset_oracle(&env, &b, &cfg(&b, &a));
        let mut cache = Cache::new_view(&env);
        crate::oracle::token_price(&mut cache, &a);
    });
}
