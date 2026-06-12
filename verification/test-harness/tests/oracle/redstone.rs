use controller::types::{
    ControllerKey, MarketConfig, OracleSourceConfig, OracleSourceConfigOption,
};
use soroban_sdk::{Address, String};
use test_harness::oracle::redstone::register_redstone_adapter;
use test_harness::{usd, usdc_preset, LendingTest, ALICE, DEFAULT_TOLERANCE};

fn configure_usdc_with_redstone_single(t: &LendingTest, redstone: &Address, feed_id: &String) {
    let asset = t.resolve_asset("USDC");
    let cfg = test_harness::redstone_single_config(
        redstone,
        feed_id,
        DEFAULT_TOLERANCE.first_upper_bps,
        DEFAULT_TOLERANCE.last_upper_bps,
    );
    t.configure_market_oracle(&asset, &cfg);
}

#[test]
fn test_redstone_single_source_market_works() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    let feed_id = String::from_str(&t.env, "USDC");
    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1))]);

    configure_usdc_with_redstone_single(&t, &redstone, &feed_id);

    t.supply(ALICE, "USDC", 1_000.0);
    t.assert_supply_near(ALICE, "USDC", 1_000.0, 1.0);
}

#[test]
fn test_reflector_primary_redstone_anchor_market_works() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_asset("USDC");
    let feed_id = String::from_str(&t.env, "USDC");
    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1))]);

    let cfg = test_harness::reflector_primary_redstone_anchor_config(
        &t.mock_reflector,
        &asset,
        &redstone,
        &feed_id,
        DEFAULT_TOLERANCE.first_upper_bps,
        DEFAULT_TOLERANCE.last_upper_bps,
    );
    t.configure_market_oracle(&asset, &cfg);

    let assets = soroban_sdk::Vec::from_array(&t.env, [asset]);
    let view = t
        .ctrl_client()
        .get_all_market_indexes_detailed(&assets)
        .get(0)
        .unwrap();
    assert_eq!(view.price_wad, usd(1));
    assert!(view.within_first_tolerance);
    assert!(view.within_second_tolerance);
}

#[test]
fn test_redstone_anchor_uses_source_specific_stale_window() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_asset("USDC");
    let feed_id = String::from_str(&t.env, "USDC");
    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1))]);
    let client = test_harness::mock_redstone::MockRedStonePriceFeedClient::new(&t.env, &redstone);
    let stale_for_market_ms = t.env.ledger().timestamp().saturating_sub(950) * 1000;
    client.set_price_data(
        &feed_id,
        &usd(1),
        &stale_for_market_ms,
        &stale_for_market_ms,
    );

    let cfg = test_harness::reflector_primary_redstone_anchor_config_with_anchor_stale(
        &t.mock_reflector,
        &asset,
        &redstone,
        &feed_id,
        86_400,
        DEFAULT_TOLERANCE.first_upper_bps,
        DEFAULT_TOLERANCE.last_upper_bps,
    );
    t.configure_market_oracle(&asset, &cfg);

    let assets = soroban_sdk::Vec::from_array(&t.env, [asset]);
    let view = t
        .ctrl_client()
        .get_all_market_indexes_detailed(&assets)
        .get(0)
        .unwrap();
    assert_eq!(view.price_wad, usd(1));
    assert!(view.within_first_tolerance);
    assert!(view.within_second_tolerance);
}

#[test]
fn test_redstone_optional_anchor_read_failure_falls_back_for_view() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_asset("USDC");
    let feed_id = String::from_str(&t.env, "USDC");
    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1))]);

    let cfg = test_harness::reflector_primary_redstone_anchor_config(
        &t.mock_reflector,
        &asset,
        &redstone,
        &feed_id,
        DEFAULT_TOLERANCE.first_upper_bps,
        DEFAULT_TOLERANCE.last_upper_bps,
    );
    t.configure_market_oracle(&asset, &cfg);

    t.env.as_contract(&t.controller, || {
        let key = ControllerKey::Market(asset.clone());
        let mut market: MarketConfig = t.env.storage().persistent().get(&key).unwrap();
        market.oracle_config.anchor = match market.oracle_config.anchor {
            OracleSourceConfigOption::Some(OracleSourceConfig::RedStone(mut config)) => {
                config.feed_id = String::from_str(&t.env, "MISSING");
                OracleSourceConfigOption::Some(OracleSourceConfig::RedStone(config))
            }
            _ => panic!("expected redstone anchor"),
        };
        t.env.storage().persistent().set(&key, &market);
    });

    let assets = soroban_sdk::Vec::from_array(&t.env, [asset]);
    let view = t
        .ctrl_client()
        .get_all_market_indexes_detailed(&assets)
        .get(0)
        .unwrap();
    assert_eq!(view.price_wad, usd(1));
    assert!(!view.within_first_tolerance);
    assert!(!view.within_second_tolerance);
}

#[test]
fn test_redstone_anchor_outside_second_tolerance_blocks_strict_view() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_asset("USDC");
    let feed_id = String::from_str(&t.env, "USDC");
    let redstone = register_redstone_adapter(&t, &[("USDC", usd(2))]);

    let cfg = test_harness::reflector_primary_redstone_anchor_config(
        &t.mock_reflector,
        &asset,
        &redstone,
        &feed_id,
        DEFAULT_TOLERANCE.first_upper_bps,
        DEFAULT_TOLERANCE.last_upper_bps,
    );
    t.configure_market_oracle(&asset, &cfg);

    let assets = soroban_sdk::Vec::from_array(&t.env, [asset]);
    let view = t
        .ctrl_client()
        .get_all_market_indexes_detailed(&assets)
        .get(0)
        .unwrap();
    assert_eq!(view.price_wad, usd(1));
    assert!(!view.within_second_tolerance);
}

// Runtime path: configure-time succeeds (price is set), then the feed is
// removed before a price read, exercising the `Err` branch in
// `try_read_price_data_for_feed` at controller/src/oracle/providers/redstone.rs.
#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn test_redstone_runtime_missing_price_panics_with_invalid_ticker() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    let feed_id = String::from_str(&t.env, "USDC");
    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1))]);

    configure_usdc_with_redstone_single(&t, &redstone, &feed_id);

    // Wipe the price entry out of the mock's temporary storage so the next
    // `read_price_data_for_feed` returns Err.
    t.env.as_contract(&redstone, || {
        let key = test_harness::mock_redstone::MockKey::PriceData(feed_id.clone());
        t.env.storage().temporary().remove(&key);
    });

    // Supply triggers a primary-source price read on the USDC market.
    t.supply(ALICE, "USDC", 1_000.0);
}
