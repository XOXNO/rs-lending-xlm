use super::*;
use common::constants::{RAY, WAD};
use common::types::{
    AccountPositionRaw, MarketIndexRaw, MarketOracleConfig, MarketOracleConfigOption,
    OracleAssetRef, OraclePriceFluctuation, OracleReadMode, OracleSourceConfig,
    OracleSourceConfigOption, OracleStrategy, PositionLimits, PositionMode, ReflectorBase,
    ReflectorSourceConfig, SpokeAssetConfig, SpokeConfig, SpokeUsageRaw,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Map};

const UNIT: i128 = 10_000_000;

fn ctx(
    supplied_tokens: i128,
    borrowed_tokens: i128,
    cash: i128,
    max_utilization: Ray,
) -> MarketLimitCtx {
    MarketLimitCtx {
        supplied: Ray::from_asset(supplied_tokens * UNIT, 7),
        borrowed: Ray::from_asset(borrowed_tokens * UNIT, 7),
        cash,
        max_utilization,
        supply_index: Ray::ONE,
        decimals: 7,
        borrow_index: Ray::ONE,
    }
}

fn default_spoke(_env: &Env) -> SpokeConfig {
    SpokeConfig {
        is_deprecated: false,
        liquidation_target_hf_wad: 1_020_000_000_000_000_000,
        hf_for_max_bonus_wad: 510_000_000_000_000_000,
        liquidation_bonus_factor_bps: 10_000,
    }
}

fn spoke_listing(borrow_cap: i128) -> SpokeAssetConfig {
    SpokeAssetConfig {
        is_collateralizable: true,
        is_borrowable: true,
        paused: false,
        frozen: false,
        loan_to_value: 7_500,
        liquidation_threshold: 8_000,
        liquidation_bonus: 500,
        liquidation_fees: 100,
        supply_cap: 0,
        borrow_cap,
        oracle_override: MarketOracleConfigOption::None,
    }
}

/// Rich collateral account (1000 tokens at $1, unit indexes) so the risk
/// gates never bind, priced through a real single-source Reflector config.
fn borrower_fixture(env: &Env) -> (Address, HubAssetKey, Account) {
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
            scaled_amount: Ray::from_asset(1_000 * UNIT, 7).raw(),
            liquidation_threshold: 9_000,
            liquidation_bonus: 500,
            loan_to_value: 8_500,
            liquidation_fees: 100,
        },
    );
    let account = Account {
        owner: Address::generate(env),
        spoke_id: 1,
        mode: PositionMode::Normal,
        supply_positions,
        borrow_positions: Map::new(env),
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
    env.as_contract(&contract, || {
        crate::storage::set_asset_oracle(env, &asset, &config);
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
    cache
}

// Re-borrowing an already-held asset needs no free position slot: the slot
// limit only gates NEW borrowed assets.
#[test]
fn account_can_reborrow_held_asset_at_position_limit() {
    let env = Env::default();
    let contract = env.register(crate::Controller, (Address::generate(&env),));
    env.as_contract(&contract, || {
        let spoke_id = 1u32;
        crate::storage::set_spoke(&env, spoke_id, &default_spoke(&env));
        let hub = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };
        crate::storage::set_spoke_asset(&env, spoke_id, &hub, &spoke_listing(0));
        crate::storage::set_position_limits(
            &env,
            &PositionLimits {
                max_supply_positions: 1,
                max_borrow_positions: 1,
            },
        );

        let mut borrow_positions = Map::new(&env);
        borrow_positions.set(
            hub.clone(),
            DebtPositionRaw {
                scaled_amount: Ray::from_asset(UNIT, 7).raw(),
            },
        );
        let account = Account {
            owner: Address::generate(&env),
            spoke_id,
            mode: PositionMode::Normal,
            supply_positions: Map::new(&env),
            borrow_positions,
        };

        let mut cache = Cache::new_view(&env);
        assert!(account_can_borrow_asset(&env, &mut cache, &account, &hub));
    });
}

// Headroom is exactly cap minus usage under unit indexes.
#[test]
fn spoke_borrow_cap_headroom_is_cap_minus_usage() {
    let env = Env::default();
    let contract = env.register(crate::Controller, (Address::generate(&env),));
    env.as_contract(&contract, || {
        let spoke_id = 1u32;
        crate::storage::set_spoke(&env, spoke_id, &default_spoke(&env));
        let hub = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };
        crate::storage::set_spoke_asset(&env, spoke_id, &hub, &spoke_listing(100 * UNIT));
        crate::storage::set_spoke_usage(
            &env,
            spoke_id,
            &hub,
            &SpokeUsageRaw {
                supplied_scaled_ray: 0,
                borrowed_scaled_ray: Ray::from_asset(40 * UNIT, 7).raw(),
            },
        );

        let account = Account {
            owner: Address::generate(&env),
            spoke_id,
            mode: PositionMode::Normal,
            supply_positions: Map::new(&env),
            borrow_positions: Map::new(&env),
        };
        let market = ctx(1_000, 0, 1_000 * UNIT, Ray::ONE);
        let mut cache = Cache::new_view(&env);
        let headroom = spoke_borrow_cap_headroom(&env, &mut cache, &account, &hub, &market);
        assert_eq!(headroom, 60 * UNIT);
    });
}

// Pool liquidity gate: the full cash balance is borrowable, one unit more
// is not.
#[test]
fn borrow_ok_cash_gate_is_inclusive() {
    let env = Env::default();
    let (contract, hub, account) = borrower_fixture(&env);
    env.as_contract(&contract, || {
        let mut cache = seeded_cache(&env, &hub);
        let market = ctx(1_000, 0, 100 * UNIT, Ray::ONE);
        assert!(borrow_ok(
            &env,
            &mut cache,
            &account,
            &hub,
            &market,
            100 * UNIT
        ));
        assert!(!borrow_ok(
            &env,
            &mut cache,
            &account,
            &hub,
            &market,
            100 * UNIT + 1
        ));
    });
}

// Utilization gate: landing exactly on the ceiling is allowed, above is
// rejected, and an uncapped market skips the gate entirely even past 100 %.
#[test]
fn borrow_ok_utilization_gate_boundaries() {
    let env = Env::default();
    let (contract, hub, account) = borrower_fixture(&env);
    env.as_contract(&contract, || {
        let mut cache = seeded_cache(&env, &hub);

        // 40 borrowed of 100 supplied at a 50 % cap: +10 hits the cap exactly.
        let capped = ctx(100, 40, 1_000 * UNIT, Ray::from(RAY / 2));
        assert!(borrow_ok(
            &env,
            &mut cache,
            &account,
            &hub,
            &capped,
            10 * UNIT
        ));
        assert!(!borrow_ok(
            &env,
            &mut cache,
            &account,
            &hub,
            &capped,
            10 * UNIT + 1
        ));

        // Uncapped market: even a borrow pushing utilization past 100 % is
        // not the utilization gate's concern.
        let uncapped = ctx(100, 95, 1_000 * UNIT, Ray::ONE);
        assert!(borrow_ok(
            &env,
            &mut cache,
            &account,
            &hub,
            &uncapped,
            10 * UNIT
        ));
    });
}

// Spoke borrow-cap gate: filling the cap exactly is allowed, one unit more
// is rejected.
#[test]
fn borrow_ok_spoke_cap_is_inclusive() {
    let env = Env::default();
    let (contract, hub, account) = borrower_fixture(&env);
    env.as_contract(&contract, || {
        crate::storage::set_spoke_asset(&env, account.spoke_id, &hub, &spoke_listing(100 * UNIT));
        crate::storage::set_spoke_usage(
            &env,
            account.spoke_id,
            &hub,
            &SpokeUsageRaw {
                supplied_scaled_ray: 0,
                borrowed_scaled_ray: Ray::from_asset(40 * UNIT, 7).raw(),
            },
        );
        let mut cache = seeded_cache(&env, &hub);
        let market = ctx(1_000, 0, 1_000 * UNIT, Ray::ONE);
        assert!(borrow_ok(
            &env,
            &mut cache,
            &account,
            &hub,
            &market,
            60 * UNIT
        ));
        assert!(!borrow_ok(
            &env,
            &mut cache,
            &account,
            &hub,
            &market,
            60 * UNIT + 1
        ));
    });
}
