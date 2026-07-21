use super::*;

// `pool_partial_cap` is the analytical seed for the partial-withdraw search.
// The settlement loop can recover a wrong seed through binary search, so the
// exact analytic values are pinned here at the unit level.

use common::constants::{RAY, WAD};

fn ctx(
    supplied_tokens: i128,
    borrowed_tokens: i128,
    cash: i128,
    max_utilization: Ray,
) -> MarketLimitCtx {
    MarketLimitCtx {
        supplied: Ray::from_asset(supplied_tokens * 10_000_000, 7),
        borrowed: Ray::from_asset(borrowed_tokens * 10_000_000, 7),
        cash,
        max_utilization,
        supply_index: Ray::ONE,
        decimals: 7,
        borrow_index: Ray::ONE,
    }
}

// With no utilization ceiling the cap is pool cash bounded by the request.
#[test]
fn pool_partial_cap_is_cash_bound_without_utilization_ceiling() {
    let env = Env::default();
    let market = ctx(1_000, 800, 500 * 10_000_000, Ray::ONE);
    assert_eq!(
        market.pool_partial_cap(&env, 1_000 * 10_000_000),
        500 * 10_000_000
    );
}

// A 50 % ceiling on 1000 supplied / 400 borrowed leaves exactly 200 tokens
// of withdrawable headroom (min supplied = 400 / 0.5 = 800).
#[test]
fn pool_partial_cap_respects_utilization_headroom() {
    let env = Env::default();
    let market = ctx(1_000, 400, 1_000 * 10_000_000, Ray::from(RAY / 2));
    assert_eq!(
        market.pool_partial_cap(&env, 1_000 * 10_000_000),
        200 * 10_000_000
    );
}

// Already above the ceiling: no partial withdrawal is possible.
#[test]
fn pool_partial_cap_is_zero_when_pool_sits_above_ceiling() {
    let env = Env::default();
    let market = ctx(1_000, 600, 1_000 * 10_000_000, Ray::from(RAY / 2));
    assert_eq!(market.pool_partial_cap(&env, 1_000 * 10_000_000), 0);
}

// Pool-state replica boundaries: paying out the full cash balance is fine,
// paying one unit more (or violating either leg independently) is not.
#[test]
fn pool_state_ok_boundaries() {
    let env = Env::default();
    let market = ctx(1_000, 0, 500 * 10_000_000, Ray::ONE);

    let out = Ray::from_asset(500 * 10_000_000, 7);
    assert!(market.pool_state_ok(&env, out, 500 * 10_000_000));
    assert!(!market.pool_state_ok(&env, out, 500 * 10_000_000 + 1));

    // Burning more shares than supplied fails even with cash available.
    let over_supplied = Ray::from_asset(1_001 * 10_000_000, 7);
    assert!(!market.pool_state_ok(&env, over_supplied, 1));
}

mod gates {
    use super::*;
    use common::types::{
        AccountPositionRaw, DebtPositionRaw, MarketIndexRaw, MarketOracleConfig, OracleAssetRef,
        OraclePriceFluctuation, OracleReadMode, OracleSourceConfig, OracleSourceConfigOption,
        OracleStrategy, PositionMode, ReflectorBase, ReflectorSourceConfig,
    };
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Map};

    const UNIT: i128 = 10_000_000;

    /// 100-token supply at $1 (unit indexes) with position-stamped risk
    /// ratios, plus a same-asset debt of `debt_tokens`.
    fn fixture(
        env: &Env,
        ltv_bps: u32,
        thr_bps: u32,
        debt_tokens: i128,
    ) -> (Address, HubAssetKey, Account) {
        use mock_oracle::{
            MockReflectorOracle, MockReflectorOracleClient, ReflectorAsset as MockAsset,
        };

        let contract = env.register(crate::Controller, (Address::generate(env),));
        let oracle_id = env.register(MockReflectorOracle, ());
        let asset = Address::generate(env);
        MockReflectorOracleClient::new(env, &oracle_id)
            .set_price(&MockAsset::Stellar(asset.clone()), &WAD);

        let hub = HubAssetKey {
            hub_id: 0,
            asset: asset.clone(),
        };
        let mut supply_positions = Map::new(env);
        supply_positions.set(
            hub.clone(),
            AccountPositionRaw {
                scaled_amount: Ray::from_asset(100 * UNIT, 7).raw(),
                liquidation_threshold: thr_bps,
                liquidation_bonus: 500,
                loan_to_value: ltv_bps,
                liquidation_fees: 100,
            },
        );
        let mut borrow_positions = Map::new(env);
        borrow_positions.set(
            hub.clone(),
            DebtPositionRaw {
                scaled_amount: Ray::from_asset(debt_tokens * UNIT, 7).raw(),
            },
        );
        let account = Account {
            owner: Address::generate(env),
            spoke_id: 1,
            mode: PositionMode::Normal,
            supply_positions,
            borrow_positions,
        };

        let config = MarketOracleConfig {
            asset_decimals: 7,
            max_price_stale_seconds: 900,
            tolerance: OraclePriceFluctuation {
                upper_ratio_bps: 10_500,
                lower_ratio_bps: 9_500,
            },
            strategy: OracleStrategy::Single,
            primary: OracleSourceConfig::Reflector(ReflectorSourceConfig {
                contract: oracle_id,
                asset: OracleAssetRef::Stellar(asset.clone()),
                read_mode: OracleReadMode::Spot,
                decimals: 14,
                resolution_seconds: 300,
                base: ReflectorBase::Usd,
            }),
            anchor: OracleSourceConfigOption::None,
            min_sanity_price_wad: 0,
            max_sanity_price_wad: i128::MAX,
        };
        let aggregator = env.register(price_aggregator::PriceAggregator, (Address::generate(env),));
        price_aggregator::PriceAggregatorClient::new(env, &aggregator)
            .seed_asset_oracle(&asset, &config);
        env.as_contract(&contract, || {
            crate::storage::set_price_aggregator(env, &aggregator);
        });
        (contract, hub, account)
    }

    fn seeded_cache(env: &Env, hub: &HubAssetKey) -> Cache {
        let mut cache = Cache::new_view(env);
        cache.put_market_index(
            hub,
            &MarketIndexRaw {
                borrow_index: RAY,
                supply_index: RAY,
            },
        );
        cache.set_prices(crate::external::price_aggregator::fetch_prices(
            env,
            &soroban_sdk::vec![env, hub.asset.clone()],
        ));
        cache
    }

    // Both replica gates are strict `<`: sitting exactly on the weighted or
    // LTV collateral is still healthy.
    #[test]
    fn account_gates_pass_at_exact_boundaries() {
        let env = Env::default();
        // Weighted boundary: threshold 0.8 -> weighted $80 == debt $80,
        // with LTV headroom above (position-stamped 100 %).
        let (contract, hub, account) = fixture(&env, 10_000, 8_000, 80);
        env.as_contract(&contract, || {
            let mut cache = seeded_cache(&env, &hub);
            assert!(account_gates_ok(&env, &mut cache, &account));
        });

        // LTV boundary: LTV 0.75 -> $75 == debt $75, weighted above.
        let (contract, hub, account) = fixture(&env, 7_500, 8_000, 75);
        env.as_contract(&contract, || {
            let mut cache = seeded_cache(&env, &hub);
            assert!(account_gates_ok(&env, &mut cache, &account));
        });
    }

    #[test]
    fn account_gates_fail_below_weighted_collateral() {
        let env = Env::default();
        let (contract, hub, account) = fixture(&env, 10_000, 8_000, 81);
        env.as_contract(&contract, || {
            let mut cache = seeded_cache(&env, &hub);
            assert!(!account_gates_ok(&env, &mut cache, &account));
        });
    }

    // A configured min-borrow floor binds: LTV collateral under the floor
    // fails even with healthy gates.
    #[test]
    fn account_gates_fail_under_min_borrow_floor() {
        let env = Env::default();
        let (contract, hub, account) = fixture(&env, 7_500, 8_000, 10);
        env.as_contract(&contract, || {
            crate::storage::set_min_borrow_collateral_usd_wad(&env, 100 * WAD);
            let mut cache = seeded_cache(&env, &hub);
            // LTV collateral $75 < floor $100.
            assert!(!account_gates_ok(&env, &mut cache, &account));
        });
    }
}
