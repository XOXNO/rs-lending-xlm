use super::*;
use common::constants::{RAY, WAD};
use common::types::{Account, PriceFeed};
use common::types::{
    AccountPositionRaw, DebtPositionRaw, MarketIndexRaw, MarketOracleConfig, OracleAssetRef,
    OraclePriceFluctuation, OracleReadMode, OracleSourceConfig, OracleSourceConfigOption,
    OracleStrategy, PositionMode, PriceFeedRaw, ReflectorBase, ReflectorSourceConfig,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Map};

const UNIT: i128 = 10_000_000;

fn ctx(supplied_tokens: i128, borrowed_tokens: i128, cash: i128) -> MarketLimitCtx {
    MarketLimitCtx {
        supplied: Ray::from_asset(supplied_tokens * UNIT, 7),
        borrowed: Ray::from_asset(borrowed_tokens * UNIT, 7),
        cash,
        max_utilization: Ray::ONE,
        supply_index: Ray::ONE,
        decimals: 7,
        borrow_index: Ray::ONE,
    }
}

fn usd_feed(price_wad: i128) -> PriceFeed {
    (&PriceFeedRaw {
        price_wad,
        asset_decimals: 7,
        timestamp: 0,
    })
        .into()
}

// $100 at $2/token with 7 decimals is exactly 50 tokens.
#[test]
fn usd_wad_to_token_cap_converts_exactly() {
    let env = Env::default();
    let cap = usd_wad_to_token_cap(&env, Wad::from(100 * WAD), usd_feed(2 * WAD), 7);
    assert_eq!(cap, 50 * UNIT);
}

// A zero price must return a zero cap instead of dividing by zero.
#[test]
fn usd_wad_to_token_cap_zero_price_is_zero() {
    let env = Env::default();
    assert_eq!(
        usd_wad_to_token_cap(&env, Wad::from(100 * WAD), usd_feed(0), 7),
        0
    );
}

fn debt_free_account(env: &Env) -> Account {
    Account {
        owner: Address::generate(env),
        spoke_id: 1,
        mode: PositionMode::Normal,
        supply_positions: Map::new(env),
        borrow_positions: Map::new(env),
    }
}

fn position(scaled_tokens: i128, ltv_bps: u32, thr_bps: u32) -> AccountPosition {
    (&AccountPositionRaw {
        scaled_amount: Ray::from_asset(scaled_tokens * UNIT, 7).raw(),
        liquidation_threshold: thr_bps,
        liquidation_bonus: 500,
        loan_to_value: ltv_bps,
        liquidation_fees: 100,
    })
        .into()
}

// A debt-free account's analytical cap is the pool cap verbatim.
#[test]
fn analytical_partial_cap_debt_free_returns_pool_cap() {
    let env = Env::default();
    let contract = env.register(crate::Controller, (Address::generate(&env),));
    env.as_contract(&contract, || {
        let account = debt_free_account(&env);
        let hub = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };
        let market = ctx(1_000, 0, 500 * UNIT);
        let pos = position(1_000, 7_500, 8_000);
        let mut cache = Cache::new_view(&env);
        let cap = analytical_partial_cap(
            &env,
            &mut cache,
            &account,
            &hub,
            &pos,
            &market,
            1_000 * UNIT,
        );
        assert_eq!(cap, 500 * UNIT);
    });
}

/// Indebted account priced through a real single-source Reflector config:
/// one supply position (100 tokens at $1) and one debt position on the same
/// hub-asset, both under unit indexes.
fn indebted_fixture(
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
    if debt_tokens > 0 {
        borrow_positions.set(
            hub.clone(),
            DebtPositionRaw {
                scaled_amount: Ray::from_asset(debt_tokens * UNIT, 7).raw(),
            },
        );
    }
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

// $100 collateral at LTV 0.5 / threshold 0.8 with $40 debt: LTV slack $10
// caps the partial at ceil(10 / 0.5) = $20 -> 20 tokens.
#[test]
fn risk_partial_cap_is_bounded_by_ltv_slack() {
    let env = Env::default();
    let (contract, hub, account) = indebted_fixture(&env, 5_000, 8_000, 40);
    env.as_contract(&contract, || {
        let mut cache = seeded_cache(&env, &hub);
        let pos = position(100, 5_000, 8_000);
        let market = ctx(1_000, 0, 1_000 * UNIT);
        let cap = risk_partial_cap(&env, &mut cache, &account, &hub, &pos, &market, 100 * UNIT);
        assert_eq!(cap, 20 * UNIT);
    });
}

// Debt equal to both weighted and LTV collateral leaves zero slack: no
// partial is allowed.
#[test]
fn risk_partial_cap_zero_slack_is_zero() {
    let env = Env::default();
    let (contract, hub, account) = indebted_fixture(&env, 8_000, 8_000, 80);
    env.as_contract(&contract, || {
        let mut cache = seeded_cache(&env, &hub);
        let pos = position(100, 8_000, 8_000);
        let market = ctx(1_000, 0, 1_000 * UNIT);
        let cap = risk_partial_cap(&env, &mut cache, &account, &hub, &pos, &market, 100 * UNIT);
        assert_eq!(cap, 0);
    });
}

// The settlement pipeline over a cash-bound market: the exact cash cap is
// the answer, from the analytic seed, the settle loop, and a raw binary
// search alike.
#[test]
fn settle_and_binary_search_agree_on_cash_bound_partial() {
    let env = Env::default();
    let contract = env.register(crate::Controller, (Address::generate(&env),));
    env.as_contract(&contract, || {
        let mut account = debt_free_account(&env);
        let hub = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };
        account.supply_positions.set(
            hub.clone(),
            AccountPositionRaw {
                scaled_amount: Ray::from_asset(1_000 * UNIT, 7).raw(),
                liquidation_threshold: 8_000,
                liquidation_bonus: 500,
                loan_to_value: 7_500,
                liquidation_fees: 100,
            },
        );
        let market = ctx(1_000, 0, 500 * UNIT);
        let pos_scaled = Ray::from_asset(1_000 * UNIT, 7);
        let ceiling = 1_000 * UNIT - 1;

        let mut cache = Cache::new_view(&env);
        let settled = settle_partial_max(
            &env,
            &mut cache,
            &account,
            &hub,
            &market,
            pos_scaled,
            500 * UNIT,
            ceiling,
        );
        assert_eq!(settled, 500 * UNIT);

        // A candidate above the feasible max settles down to it.
        let settled_high = settle_partial_max(
            &env,
            &mut cache,
            &account,
            &hub,
            &market,
            pos_scaled,
            500 * UNIT + 2,
            ceiling,
        );
        assert_eq!(settled_high, 500 * UNIT);

        let searched = binary_search_partial(
            &env, &mut cache, &account, &hub, &market, pos_scaled, 0, ceiling,
        );
        assert_eq!(searched, 500 * UNIT);
    });
}

// Withdrawing the entire position (shares equal at the half-up conversion)
// routes through the full-close replica and is feasible in a liquid pool.
#[test]
fn partial_ok_full_position_boundary_is_feasible() {
    let env = Env::default();
    let contract = env.register(crate::Controller, (Address::generate(&env),));
    env.as_contract(&contract, || {
        let mut account = debt_free_account(&env);
        let hub = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };
        account.supply_positions.set(
            hub.clone(),
            AccountPositionRaw {
                scaled_amount: Ray::from_asset(100 * UNIT, 7).raw(),
                liquidation_threshold: 8_000,
                liquidation_bonus: 500,
                loan_to_value: 7_500,
                liquidation_fees: 100,
            },
        );
        let market = ctx(1_000, 0, 1_000 * UNIT);
        let pos_scaled = Ray::from_asset(100 * UNIT, 7);

        let mut cache = Cache::new_view(&env);
        assert!(partial_ok(
            &env,
            &mut cache,
            &account,
            &hub,
            &market,
            pos_scaled,
            100 * UNIT
        ));
        // Anything above the position is not a feasible partial.
        assert!(!partial_ok(
            &env,
            &mut cache,
            &account,
            &hub,
            &market,
            pos_scaled,
            100 * UNIT + 1
        ));
    });
}

// The full-close replica must actually consult pool state: a cash-starved
// pool cannot pay out the position, a liquid one can.
#[test]
fn full_close_ok_tracks_pool_cash() {
    let env = Env::default();
    let contract = env.register(crate::Controller, (Address::generate(&env),));
    env.as_contract(&contract, || {
        let mut account = debt_free_account(&env);
        let hub = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };
        account.supply_positions.set(
            hub.clone(),
            AccountPositionRaw {
                scaled_amount: Ray::from_asset(1_000 * UNIT, 7).raw(),
                liquidation_threshold: 8_000,
                liquidation_bonus: 500,
                loan_to_value: 7_500,
                liquidation_fees: 100,
            },
        );
        let pos_scaled = Ray::from_asset(1_000 * UNIT, 7);
        let mut cache = Cache::new_view(&env);

        let starved = ctx(1_000, 0, 500 * UNIT);
        assert!(!full_close_ok(
            &env, &mut cache, &account, &hub, &starved, pos_scaled
        ));

        let liquid = ctx(1_000, 0, 1_000 * UNIT);
        assert!(full_close_ok(
            &env, &mut cache, &account, &hub, &liquid, pos_scaled
        ));
    });
}
