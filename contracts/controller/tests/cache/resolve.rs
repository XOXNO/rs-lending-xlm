use super::*;
use crate::Controller;
use common::types::MarketOracleConfigOption;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

fn spoke_asset_with_override(oracle_override: MarketOracleConfigOption) -> SpokeAssetConfig {
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
        oracle_override,
    }
}

// Oracle resolution is token-rooted: it returns the `AssetOracle` entry that
// `set_market_oracle_config` writes, independent of any spoke.
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
        assert_eq!(cache.resolve_oracle_config(&asset), base);
    });
}

// Pricing is token-rooted: a spoke-asset listing never diverts oracle resolution
// from the token's `AssetOracle` base, regardless of the listing's contents.
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
            &spoke_asset_with_override(MarketOracleConfigOption::None),
        );

        let mut cache = Cache::new_view(&env);
        cache.ensure_spoke_context(spoke_id);
        assert_eq!(cache.resolve_oracle_config(&asset), base);
    });
}

// The price-resolution stack reverts on re-entry of an asset already being
// priced. This is the guard `token_price` uses to break a quote/anchor cycle
// (asset A quoted in B, B quoted in A) before the shadow stack traps. The end-
// to-end cyclic config would require a live Reflector read to reach the second
// hop; here we exercise the guard directly at its mechanism (token_price wires
// it in unconditionally). OracleError::OracleCycleDetected = 225.
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
        // Re-entering the same asset mid-resolution is exactly the cycle.
        cache.enter_price_resolution(&asset);
    });
}

// A legitimate (acyclic) resolution DAG never trips the guard: distinct assets
// nest freely, and an asset can be priced again once its prior resolution has
// popped off the stack — so the guard adds no false positives.
#[test]
fn price_resolution_allows_distinct_and_sequential() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    let a = Address::generate(&env);
    let b = Address::generate(&env);

    env.as_contract(&contract_id, || {
        let mut cache = Cache::new_view(&env);
        // Nested distinct assets: A resolving, then B resolving within it.
        cache.enter_price_resolution(&a);
        cache.enter_price_resolution(&b);
        cache.exit_price_resolution(); // B done
        cache.exit_price_resolution(); // A done
                                       // A can be resolved again now that the stack is clear — not a cycle.
        cache.enter_price_resolution(&a);
        cache.exit_price_resolution();
    });
}

// End-to-end: two markets each anchored (via a Reflector Quoted source) in the
// other, both with USD primaries — the exact shape that passes config-time
// validation (which only inspects the quote's primary). Driving token_price on
// one recurses A -> anchor Quoted(B) -> token_price(B) -> anchor Quoted(A) ->
// token_price(A) through the real compose/reflector path, which the guard traps
// with OracleCycleDetected (#225) instead of exhausting the shadow stack.
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

    // Positive price for both assets so the Reflector spot reads succeed and
    // resolution proceeds into the quote (anchor) legs.
    let mock = MockReflectorOracleClient::new(&env, &oracle_id);
    let one_usd_wad: i128 = 1_000_000_000_000_000_000;
    mock.set_price(&MockAsset::Stellar(a.clone()), &one_usd_wad);
    mock.set_price(&MockAsset::Stellar(b.clone()), &one_usd_wad);

    // USD primary + an anchor quoted in `quote` — a mutual pair is a cycle.
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
