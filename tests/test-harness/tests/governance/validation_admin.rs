//! IRM, asset-config, and oracle-config validation through the governance
//! client, with real mock oracles probed in-path.

use controller::constants::{BPS, MAX_REASONABLE_PRICE_WAD, RAY};
use controller::types::{AssetOracle, InterestRateModel, OracleReadMode};
use governance::op::{
    AdminOperation, ConfigureAssetOracleArgs, SpokeAssetArgs, UpgradePoolParamsArgs,
};
use soroban_sdk::{String, Symbol, Vec};
use test_harness::{
    hub_asset, usdc_preset, LendingTest, DEFAULT_TOLERANCE, HARNESS_HUB, HARNESS_SPOKE,
};

// `InterestRateModel::verify` invariants, driven via
// `upgrade_liquidity_pool_params`, which validates before forwarding.

fn baseline_irm() -> InterestRateModel {
    InterestRateModel {
        max_borrow_rate: 2 * RAY,
        base_borrow_rate: RAY / 100,
        slope1: RAY * 4 / 100,
        slope2: RAY * 10 / 100,
        slope3: RAY * 150 / 100,
        mid_utilization: RAY * 50 / 100,
        optimal_utilization: RAY * 80 / 100,
        max_utilization: RAY * 95 / 100,
        reserve_factor: 1000,
        is_flashloanable: false,
        flashloan_fee: 0,
    }
}

// base_borrow_rate < 0 -> BaseRateNegative (#128).
#[test]
#[should_panic(expected = "Error(Contract, #128)")]
fn test_validate_irm_rejects_negative_base_rate() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let admin = t.admin();
    let mut irm = baseline_irm();
    irm.base_borrow_rate = -1;
    t.gov_client().execute_immediate(
        &admin,
        &AdminOperation::UpgradeLiquidityPoolParams(UpgradePoolParamsArgs {
            hub_asset: hub_asset(asset),
            params: irm,
        }),
    );
}

// mid_utilization <= 0 rejects InvalidUtilRange (#117).
#[test]
#[should_panic(expected = "Error(Contract, #117)")]
fn test_validate_irm_rejects_zero_mid_utilization() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let admin = t.admin();
    let mut irm = baseline_irm();
    irm.mid_utilization = 0;
    t.gov_client().execute_immediate(
        &admin,
        &AdminOperation::UpgradeLiquidityPoolParams(UpgradePoolParamsArgs {
            hub_asset: hub_asset(asset),
            params: irm,
        }),
    );
}

// optimal_utilization <= mid_utilization rejects #117.
#[test]
#[should_panic(expected = "Error(Contract, #117)")]
fn test_validate_irm_rejects_optimal_not_above_mid() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let admin = t.admin();
    let mut irm = baseline_irm();
    irm.optimal_utilization = irm.mid_utilization;
    t.gov_client().execute_immediate(
        &admin,
        &AdminOperation::UpgradeLiquidityPoolParams(UpgradePoolParamsArgs {
            hub_asset: hub_asset(asset),
            params: irm,
        }),
    );
}

// optimal_utilization >= RAY rejects OptUtilTooHigh (#118).
#[test]
#[should_panic(expected = "Error(Contract, #118)")]
fn test_validate_irm_rejects_optimal_at_or_above_ray() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let admin = t.admin();
    let mut irm = baseline_irm();
    irm.optimal_utilization = RAY;
    t.gov_client().execute_immediate(
        &admin,
        &AdminOperation::UpgradeLiquidityPoolParams(UpgradePoolParamsArgs {
            hub_asset: hub_asset(asset),
            params: irm,
        }),
    );
}

// reserve_factor >= BPS rejects InvalidReserveFactor (#119).
#[test]
#[should_panic(expected = "Error(Contract, #119)")]
fn test_validate_irm_rejects_reserve_factor_at_bps() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let admin = t.admin();
    let mut irm = baseline_irm();
    irm.reserve_factor = BPS as u32;
    t.gov_client().execute_immediate(
        &admin,
        &AdminOperation::UpgradeLiquidityPoolParams(UpgradePoolParamsArgs {
            hub_asset: hub_asset(asset),
            params: irm,
        }),
    );
}

// `validate_risk_bounds` invariants, driven via `edit_asset_in_spoke`.

// threshold*(1+bonus) > 100% rejects #113: a bonus large enough that
// liquidation seizure would exceed collateral is invalid (mints bad debt).
#[test]
#[should_panic(expected = "Error(Contract, #113)")]
fn test_edit_asset_in_spoke_rejects_excessive_liq_bonus() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let admin = t.admin();
    let cfg = t
        .ctrl_client()
        .get_spoke_asset(&1u32, &hub_asset(asset.clone()));
    let args = SpokeAssetArgs {
        liquidation_fees: cfg.liquidation_fees,
        hub_id: HARNESS_HUB,
        asset,
        spoke_id: HARNESS_SPOKE,
        can_collateral: cfg.is_collateralizable,
        can_borrow: cfg.is_borrowable,
        paused: false,
        frozen: false,
        // 95% threshold * (1 + 10% bonus) = 104.5% > 100%.
        ltv: 8000,
        threshold: 9500,
        bonus: 1000,
        supply_cap: cfg.supply_cap,
        borrow_cap: cfg.borrow_cap,
    };
    t.gov_client()
        .execute_immediate(&admin, &AdminOperation::EditAssetInSpoke(args));
}

// A large bonus is permitted when the threshold leaves room:
// 50% threshold * (1 + 50% bonus) = 75% <= 100%. The bonus ceiling is the
// invariant, not a flat cap.
#[test]
fn test_edit_asset_in_spoke_accepts_high_bonus_low_threshold() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let admin = t.admin();
    let cfg = t
        .ctrl_client()
        .get_spoke_asset(&1u32, &hub_asset(asset.clone()));
    let args = SpokeAssetArgs {
        liquidation_fees: 0,
        hub_id: HARNESS_HUB,
        asset: asset.clone(),
        spoke_id: HARNESS_SPOKE,
        can_collateral: cfg.is_collateralizable,
        can_borrow: cfg.is_borrowable,
        paused: false,
        frozen: false,
        ltv: 4000,
        threshold: 5000,
        bonus: 5000,
        supply_cap: cfg.supply_cap,
        borrow_cap: cfg.borrow_cap,
    };
    t.gov_client()
        .execute_immediate(&admin, &AdminOperation::EditAssetInSpoke(args));
}

// `configure_market_oracle` error paths against the live mock reflector.

fn base_oracle_config(t: &LendingTest) -> AssetOracle {
    let market = t.resolve_market("USDC");
    test_harness::reflector_primary_anchor_config(
        &t.env,
        &t.mock_reflector,
        &market.asset,
        market.price_wad,
        DEFAULT_TOLERANCE.tolerance_bps,
    )
}

fn set_primary_reflector_read_mode(cfg: &mut AssetOracle, read_mode: OracleReadMode) {
    test_harness::set_reflector_read_mode(cfg, 0, read_mode);
}

fn configure_usdc(t: &LendingTest, cfg: &AssetOracle) {
    let asset = t.resolve_market("USDC").asset.clone();
    let admin = t.admin();
    t.gov_client().execute_immediate(
        &admin,
        &AdminOperation::ConfigureAssetOracle(ConfigureAssetOracleArgs {
            key: controller::types::PriceKey::Token(asset),
            oracle: cfg.clone(),
        }),
    );
}

// max_price_stale_seconds < 60 rejects InvalidStalenessConfig (#218).
#[test]
#[should_panic(expected = "Error(Contract, #218)")]
fn test_configure_market_oracle_rejects_low_staleness() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let mut cfg = base_oracle_config(&t);
    cfg.max_price_stale_seconds = 30; // Below the 60-second floor.
    configure_usdc(&t, &cfg);
}

// max_price_stale_seconds > 86_400 rejects #218 (upper bound).
#[test]
#[should_panic(expected = "Error(Contract, #218)")]
fn test_configure_market_oracle_rejects_high_staleness() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let mut cfg = base_oracle_config(&t);
    cfg.max_price_stale_seconds = 86_401; // Above the 24-hour ceiling.
    configure_usdc(&t, &cfg);
}

// twap_records > 12 rejects TwapRecordsOutOfRange (#228).
#[test]
#[should_panic(expected = "Error(Contract, #228)")]
fn test_configure_market_oracle_rejects_excessive_twap_records() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let mut cfg = base_oracle_config(&t);
    set_primary_reflector_read_mode(&mut cfg, OracleReadMode::Twap(13));
    configure_usdc(&t, &cfg);
}

// A sourceless oracle would price nothing while still reporting a config, so
// arity is bounded on both ends: SourceCountOutOfRange (#231).
#[test]
#[should_panic(expected = "Error(Contract, #231)")]
fn test_configure_market_oracle_rejects_zero_sources() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let mut cfg = base_oracle_config(&t);
    cfg.sources = Vec::new(&t.env);
    configure_usdc(&t, &cfg);
}

// Three sources have no defined agreement rule — the band is pairwise — so the
// upper arity bound is enforced rather than silently truncating to two (#231).
#[test]
#[should_panic(expected = "Error(Contract, #231)")]
fn test_configure_market_oracle_rejects_three_sources() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let redstone = t.mock_reflector.clone();
    let mut cfg = base_oracle_config(&t);
    for name in ["BTC", "ETH"] {
        let feed_id = String::from_str(&t.env, name);
        cfg.sources
            .push_back(test_harness::redstone_source(&redstone, &feed_id));
    }
    configure_usdc(&t, &cfg);
}

// Two sources on the same contract are not two opinions: whoever controls that
// contract controls both legs, so the agreement band it is meant to police
// compares a value against itself. Under `RequireDisjoint` that is
// IndependenceNotDeclared (#232).
#[test]
#[should_panic(expected = "Error(Contract, #232)")]
fn test_configure_market_oracle_rejects_identical_sources() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let mut cfg = base_oracle_config(&t);
    cfg.sources.push_back(cfg.sources.get_unchecked(0));
    configure_usdc(&t, &cfg);
}

// Same contract and feed, differing only in the policy-only `max_stale_seconds`.
// Independence is judged on the contract address, not on config equality, so
// these are rejected (#232) even though the two structs are not byte-equal —
// the shared-writer risk is identical.
#[test]
#[should_panic(expected = "Error(Contract, #232)")]
fn test_configure_market_oracle_rejects_same_redstone_feed_distinct_max_stale() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    // Independence is judged before any live feed read, so a placeholder
    // contract address suffices; both sources share it (and the feed id) on
    // purpose so they resolve to the same underlying feed.
    let redstone = t.mock_reflector.clone();
    let feed_id = String::from_str(&t.env, "BTC");

    let mut cfg = base_oracle_config(&t);
    cfg.sources = Vec::from_array(
        &t.env,
        [
            test_harness::redstone_source_with_max_stale(&redstone, &feed_id, 600),
            test_harness::redstone_source_with_max_stale(&redstone, &feed_id, 900),
        ],
    );
    configure_usdc(&t, &cfg);
}

#[test]
#[should_panic(expected = "Error(Contract, #220)")]
fn test_configure_market_oracle_rejects_non_usd_base() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let cfg = base_oracle_config(&t);
    t.mock_reflector_client()
        .set_base_other(&Symbol::new(&t.env, "EUR"));
    configure_usdc(&t, &cfg);
}

#[test]
#[should_panic(expected = "Error(Contract, #221)")]
fn test_configure_market_oracle_rejects_bad_reflector_decimals() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let cfg = base_oracle_config(&t);
    t.mock_reflector_client().set_decimals(&19);
    configure_usdc(&t, &cfg);
}

#[test]
#[should_panic(expected = "Error(Contract, #222)")]
fn test_configure_market_oracle_rejects_bad_reflector_resolution() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let cfg = base_oracle_config(&t);
    t.mock_reflector_client().set_resolution(&0);
    configure_usdc(&t, &cfg);
}

// TWAP history problems are miss-equivalent (`read_twap` errors are `.ok()`ed
// at the provider entry), so the configure probe reverts `NoLastPrice` (#210).
#[test]
#[should_panic(expected = "Error(Contract, #210)")]
fn test_configure_market_oracle_rejects_missing_twap_history() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let cfg = base_oracle_config(&t);
    t.mock_reflector_client().set_twap_history_mode(&asset, &1);
    configure_usdc(&t, &cfg);
}

// `validate_sanity_bounds` at configure time — #224.
#[test]
#[should_panic(expected = "Error(Contract, #224)")]
fn test_configure_market_oracle_rejects_zero_min_sanity() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let mut cfg = base_oracle_config(&t);
    cfg.min_sanity_price_wad = 0;
    configure_usdc(&t, &cfg);
}

#[test]
#[should_panic(expected = "Error(Contract, #224)")]
fn test_configure_market_oracle_rejects_min_ge_max_sanity() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let mut cfg = base_oracle_config(&t);
    cfg.min_sanity_price_wad = 100;
    cfg.max_sanity_price_wad = 100;
    configure_usdc(&t, &cfg);
}

#[test]
#[should_panic(expected = "Error(Contract, #224)")]
fn test_configure_market_oracle_rejects_max_sanity_above_cap() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let mut cfg = base_oracle_config(&t);
    cfg.max_sanity_price_wad = MAX_REASONABLE_PRICE_WAD + 1;
    configure_usdc(&t, &cfg);
}
