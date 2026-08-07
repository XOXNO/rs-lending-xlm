use controller::constants::{BPS, MAX_REASONABLE_PRICE_WAD, RAY};
use controller::types::{AssetOracle, InterestRateModel, OracleReadMode};
use governance::op::{
    AdminOperation, ConfigureAssetOracleArgs, SpokeAssetArgs, UpgradePoolParamsArgs,
};
use soroban_sdk::{String, Symbol, Vec};
use test_harness::{
    assert_contract_error, errors, hub_asset, usdc_preset, LendingTest, DEFAULT_TOLERANCE,
    HARNESS_HUB, HARNESS_SPOKE,
};

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

        ltv: 8000,
        threshold: 9500,
        bonus: 1000,
        supply_cap: cfg.supply_cap,
        borrow_cap: cfg.borrow_cap,
    };
    t.gov_client()
        .execute_immediate(&admin, &AdminOperation::EditAssetInSpoke(args));
}

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

#[test]
#[should_panic(expected = "Error(Contract, #218)")]
fn test_configure_market_oracle_rejects_low_staleness() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let mut cfg = base_oracle_config(&t);
    cfg.max_price_stale_seconds = 30;
    configure_usdc(&t, &cfg);
}

#[test]
#[should_panic(expected = "Error(Contract, #218)")]
fn test_configure_market_oracle_rejects_high_staleness() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let mut cfg = base_oracle_config(&t);
    cfg.max_price_stale_seconds = 86_401;
    configure_usdc(&t, &cfg);
}

#[test]
#[should_panic(expected = "Error(Contract, #228)")]
fn test_configure_market_oracle_rejects_excessive_twap_records() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let mut cfg = base_oracle_config(&t);
    set_primary_reflector_read_mode(&mut cfg, OracleReadMode::Twap(13));
    configure_usdc(&t, &cfg);
}

#[test]
#[should_panic(expected = "Error(Contract, #231)")]
fn test_configure_market_oracle_rejects_zero_sources() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let mut cfg = base_oracle_config(&t);
    cfg.sources = Vec::new(&t.env);
    configure_usdc(&t, &cfg);
}

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

#[test]
#[should_panic(expected = "Error(Contract, #232)")]
fn test_configure_market_oracle_rejects_identical_sources() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let mut cfg = base_oracle_config(&t);
    cfg.sources.push_back(cfg.sources.get_unchecked(0));
    configure_usdc(&t, &cfg);
}

#[test]
#[should_panic(expected = "Error(Contract, #232)")]
fn test_configure_market_oracle_rejects_same_redstone_feed_distinct_max_stale() {
    let t = LendingTest::new().with_market(usdc_preset()).build();

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

#[test]
fn test_configure_market_oracle_defers_missing_twap_history_to_read_time() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let cfg = base_oracle_config(&t);
    t.mock_reflector_client().set_twap_history_mode(&asset, &1);
    configure_usdc(&t, &cfg);

    let read = t
        .price_agg_client()
        .try_prices(&soroban_sdk::vec![
            &t.env,
            controller::types::PriceKey::Token(asset)
        ])
        .map(|inner| inner.map(|_| ()).map_err(|e| e.into()))
        .unwrap_or_else(|e| Err(e.expect("expected contract error")));
    assert_contract_error(read, errors::NO_LAST_PRICE);
}

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
