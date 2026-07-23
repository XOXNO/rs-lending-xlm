use controller::types::{OracleSourceConfig, OracleSourceConfigOption};
use soroban_sdk::{Address, String};
use test_harness::oracle::redstone::register_redstone_adapter;
use test_harness::{hub_asset, usd, usdc_preset, LendingTest, ALICE, BOB, DEFAULT_TOLERANCE};

fn configure_usdc_with_redstone_single(t: &LendingTest, redstone: &Address, feed_id: &String) {
    let asset = t.resolve_asset("USDC");
    let cfg = test_harness::redstone_single_config(
        redstone,
        feed_id,
        usd(1),
        DEFAULT_TOLERANCE.tolerance_bps,
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
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    t.configure_market_oracle(&asset, &cfg);

    let assets = soroban_sdk::Vec::from_array(&t.env, [hub_asset(asset)]);
    let view = t
        .ctrl_client()
        .get_market_indexes_detailed(&assets)
        .get(0)
        .unwrap();
    // Both feeds agree at $1, so the in-band blend is the midpoint $1.
    assert_eq!(view.price_wad, usd(1));
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
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    t.configure_market_oracle(&asset, &cfg);

    let assets = soroban_sdk::Vec::from_array(&t.env, [hub_asset(asset)]);
    let view = t
        .ctrl_client()
        .get_market_indexes_detailed(&assets)
        .get(0)
        .unwrap();
    // Anchor is within its source-specific 86_400s window, so the read
    // succeeds and the in-band blend is the midpoint $1.
    assert_eq!(view.price_wad, usd(1));
}

// Soft view: a required RedStone anchor that cannot be read marks the row
// invalid (no primary-only fallback). Write-path `prices()` still fail-closed.
#[test]
fn test_redstone_anchor_read_failure_marks_view_invalid() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_asset("USDC");
    let feed_id = String::from_str(&t.env, "USDC");
    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1))]);

    let cfg = test_harness::reflector_primary_redstone_anchor_config(
        &t.mock_reflector,
        &asset,
        &redstone,
        &feed_id,
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    t.configure_market_oracle(&asset, &cfg);

    let mut oracle = t.price_agg_client().oracle_config(&asset).unwrap();
    oracle.anchor = match oracle.anchor {
        OracleSourceConfigOption::Some(OracleSourceConfig::RedStone(mut config)) => {
            config.feed_id = String::from_str(&t.env, "MISSING");
            OracleSourceConfigOption::Some(OracleSourceConfig::RedStone(config))
        }
        _ => panic!("expected redstone anchor"),
    };
    t.price_agg_client().seed_oracle_config(&asset, &oracle);

    let assets = soroban_sdk::Vec::from_array(&t.env, [hub_asset(asset)]);
    let row = t
        .ctrl_client()
        .get_market_indexes_detailed(&assets)
        .get(0)
        .unwrap();
    assert!(!row.valid, "missing anchor must not report valid");
    assert!(row.deviation, "missing dual-source anchor is deviation");
}

// Soft view: primary $1 and anchor $2 are 100% apart → deviation=true,
// valid=false. Write-path still reverts UnsafePriceNotAllowed (#205).
#[test]
fn test_redstone_anchor_outside_tolerance_marks_view_deviation() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_asset("USDC");
    let feed_id = String::from_str(&t.env, "USDC");
    let redstone = register_redstone_adapter(&t, &[("USDC", usd(2))]);

    let cfg = test_harness::reflector_primary_redstone_anchor_config(
        &t.mock_reflector,
        &asset,
        &redstone,
        &feed_id,
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    t.configure_market_oracle(&asset, &cfg);

    let assets = soroban_sdk::Vec::from_array(&t.env, [hub_asset(asset)]);
    let row = t
        .ctrl_client()
        .get_market_indexes_detailed(&assets)
        .get(0)
        .unwrap();
    assert!(!row.valid);
    assert!(row.deviation);
    assert!(!row.stale);
    assert!(row.safe_price_wad > 0);
    assert!(row.aggregator_price_wad > 0);
}

// Runtime path: configure-time succeeds (price is set), then the feed is
// removed before a price read, exercising the `Err` branch in
// `try_read_price_data_for_feed` at price-aggregator multi-feed provider.
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

    // Supply skips dust pricing; borrow prices (RiskIncreasing LTV/HF).
    t.supply(BOB, "USDC", 100_000.0);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDC", 100.0);
}
