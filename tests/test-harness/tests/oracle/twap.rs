use controller::types::OracleReadMode;
use soroban_sdk::vec;
use test_harness::{
    assert_contract_error, errors, hub_asset, usd, usd_cents, usdc_preset, LendingTest, ALICE,
};

fn setup() -> LendingTest {
    LendingTest::new().dual_source_two_asset()
}

#[test]
#[should_panic(expected = "Error(Contract, #222)")]
fn configure_rejects_twap_window_larger_than_max_stale() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let usdc = t.resolve_asset("USDC");
    t.mock_reflector_client().set_resolution(&60);
    let mut cfg = test_harness::reflector_primary_anchor_config(
        &t.env,
        &t.mock_reflector,
        &usdc,
        usd(1),
        test_harness::DEFAULT_TOLERANCE.tolerance_bps,
    );

    cfg.max_price_stale_seconds = 60;
    if let controller::types::PriceSource::Feed(mut feed) = cfg.sources.get_unchecked(0) {
        feed.max_stale_seconds = 60;
        cfg.sources
            .set(0, controller::types::PriceSource::Feed(feed));
    }

    t.configure_market_oracle(&usdc, &cfg);
}

fn try_price(t: &LendingTest, asset: &soroban_sdk::Address) -> Result<(), soroban_sdk::Error> {
    t.price_agg_client()
        .try_price(&controller::types::PriceKey::Token(asset.clone()))
        .map(|inner| inner.map(|_| ()).map_err(|e| e.into()))
        .unwrap_or_else(|e| Err(e.expect("expected contract error")))
}

#[test]
fn configure_twap_defers_out_of_band_mean_to_read_time() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let usdc = t.resolve_asset("USDC");
    t.mock_reflector_client().set_price(&usdc, &usd(1));
    t.mock_reflector_client().set_twap_price(&usdc, &usd(3));

    let mut cfg = test_harness::reflector_single_spot_config(
        &t.env,
        &t.mock_reflector,
        &usdc,
        usd(1),
        test_harness::DEFAULT_TOLERANCE.tolerance_bps,
    );
    test_harness::set_reflector_read_mode(&mut cfg, 0, OracleReadMode::Twap(3));

    t.configure_market_oracle(&usdc, &cfg);
    test_harness::assert_contract_error(
        try_price(&t, &usdc),
        test_harness::errors::SANITY_BOUND_VIOLATED,
    );
}

#[test]
fn configure_defers_nonpositive_live_reflector_price_to_read_time() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let usdc = t.resolve_asset("USDC");
    t.mock_reflector_client().set_price(&usdc, &0);
    let cfg = test_harness::reflector_primary_anchor_config(
        &t.env,
        &t.mock_reflector,
        &usdc,
        usd(1),
        test_harness::DEFAULT_TOLERANCE.tolerance_bps,
    );

    t.configure_market_oracle(&usdc, &cfg);
    test_harness::assert_contract_error(try_price(&t, &usdc), test_harness::errors::NO_LAST_PRICE);
}

#[test]
fn test_empty_twap_history_blocks_strict_borrow() {
    let mut t = setup();
    let usdc_asset = t.resolve_asset("USDC");
    t.mock_reflector_client()
        .set_twap_history_mode(&usdc_asset, &2);

    t.supply(ALICE, "USDC", 100_000.0);
    let result = t.try_borrow(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::UNSAFE_PRICE);
}

#[test]
fn test_insufficient_twap_history_blocks_strict_borrow() {
    let mut t = setup();
    let usdc_asset = t.resolve_asset("USDC");
    t.mock_reflector_client()
        .set_twap_history_mode(&usdc_asset, &3);

    t.supply(ALICE, "USDC", 100_000.0);
    let result = t.try_borrow(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::UNSAFE_PRICE);
}

#[test]
fn test_exact_minimum_twap_history_is_accepted() {
    let mut t = setup();
    let usdc_asset = t.resolve_asset("USDC");
    t.mock_reflector_client()
        .set_twap_history_mode(&usdc_asset, &6);

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    assert!(t.health_factor(ALICE) > 1.0);
}

#[test]
fn test_duplicate_timestamp_twap_blocks_strict_borrow() {
    let mut t = setup();
    let usdc_asset = t.resolve_asset("USDC");
    t.mock_reflector_client()
        .set_twap_history_mode(&usdc_asset, &7);

    t.supply(ALICE, "USDC", 100_000.0);
    assert_contract_error(t.try_borrow(ALICE, "ETH", 1.0), errors::UNSAFE_PRICE);
}

#[test]
fn test_insufficient_span_twap_blocks_strict_borrow() {
    let mut t = setup();
    let usdc_asset = t.resolve_asset("USDC");
    t.mock_reflector_client()
        .set_twap_history_mode(&usdc_asset, &8);

    t.supply(ALICE, "USDC", 100_000.0);
    assert_contract_error(t.try_borrow(ALICE, "ETH", 1.0), errors::UNSAFE_PRICE);
}

#[test]
fn test_clustered_adjacent_twap_sample_blocks_strict_borrow() {
    let mut t = setup();
    let usdc_asset = t.resolve_asset("USDC");
    t.mock_reflector_client()
        .set_twap_history_mode(&usdc_asset, &9);

    t.supply(ALICE, "USDC", 100_000.0);
    assert_contract_error(t.try_borrow(ALICE, "ETH", 1.0), errors::UNSAFE_PRICE);
}

#[test]
fn test_spot_with_cents_price_supplies_cleanly() {
    let mut t = setup();
    t.set_price("USDC", usd_cents(99));
    t.supply(ALICE, "USDC", 5_000.0);
    t.assert_supply_near(ALICE, "USDC", 5_000.0, 1.0);
}

#[test]
fn test_twap_invalid_price_blocks_strict_borrow() {
    let mut t = setup();
    let usdc_asset = t.resolve_asset("USDC");
    t.mock_reflector_client()
        .set_twap_history_mode(&usdc_asset, &4);

    t.supply(ALICE, "USDC", 100_000.0);
    let result = t.try_borrow(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::UNSAFE_PRICE);
}

#[test]
fn test_twap_stale_history_blocks_strict_borrow() {
    let mut t = setup();
    let usdc_asset = t.resolve_asset("USDC");
    t.mock_reflector_client()
        .set_twap_history_mode(&usdc_asset, &5);

    t.supply(ALICE, "USDC", 100_000.0);
    let result = t.try_borrow(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::PRICE_FEED_STALE);
}

#[test]
fn test_twap_degradation_on_view_reports_unusable() {
    let t = setup();
    let usdc_asset = t.resolve_asset("USDC");
    t.mock_reflector_client()
        .set_twap_history_mode(&usdc_asset, &2);

    let assets = soroban_sdk::Vec::from_array(&t.env, [hub_asset(usdc_asset.clone())]);
    let rows = t.ctrl_client().get_market_indexes_detailed(&assets);
    let row = rows.get(0).unwrap();
    assert_eq!(row.asset, usdc_asset);
    assert!(!row.valid);
    assert_eq!(row.price_wad, 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #210)")]
fn test_reflector_spot_missing_lastprice_panics_under_strict() {
    let mut t = setup();
    let usdc_asset = t.resolve_asset("USDC");
    let eth_asset = t.resolve_asset("ETH");

    let spot_cfg = test_harness::reflector_single_spot_config(
        &t.env,
        &t.mock_reflector,
        &eth_asset,
        usd(2_000),
        test_harness::DEFAULT_TOLERANCE.tolerance_bps,
    );
    t.price_agg_client().seed_oracle(
        &controller::types::PriceKey::Token(eth_asset.clone()),
        &spot_cfg,
    );

    let _ = usdc_asset;
    t.supply(ALICE, "USDC", 100_000.0);

    let reflector_addr = t.mock_reflector.clone();
    let eth_clone = eth_asset.clone();
    t.env.as_contract(&reflector_addr, || {
        let key = test_harness::mock_reflector::MockKey::Spot(eth_clone);
        t.env.storage().temporary().remove(&key);
    });

    t.borrow(ALICE, "ETH", 1.0);
}

#[test]
#[should_panic(expected = "Error(Contract, #219)")]
fn test_twap_zero_records_reverts_on_view() {
    let t = LendingTest::new().dual_source_two_asset();
    let usdc = t.resolve_asset("USDC");
    let mut oracle = t
        .price_agg_client()
        .oracle(&controller::types::PriceKey::Token(usdc.clone()))
        .unwrap();
    test_harness::set_reflector_read_mode(&mut oracle, 0, OracleReadMode::Twap(0));
    t.price_agg_client()
        .seed_oracle(&controller::types::PriceKey::Token(usdc.clone()), &oracle);

    let assets = vec![&t.env, hub_asset(usdc)];
    let _ = t.ctrl_client().get_market_indexes_detailed(&assets);
}

#[test]
#[should_panic(expected = "Error(Contract, #228)")]
fn test_twap_records_above_max_rejects_on_view() {
    let t = LendingTest::new().dual_source_two_asset();
    let usdc = t.resolve_asset("USDC");
    let mut oracle = t
        .price_agg_client()
        .oracle(&controller::types::PriceKey::Token(usdc.clone()))
        .unwrap();
    test_harness::set_reflector_read_mode(&mut oracle, 0, OracleReadMode::Twap(13));
    t.price_agg_client()
        .seed_oracle(&controller::types::PriceKey::Token(usdc.clone()), &oracle);

    let assets = vec![&t.env, hub_asset(usdc)];
    let _ = t.ctrl_client().get_market_indexes_detailed(&assets);
}

#[test]
fn test_dual_anchor_missing_spot_marks_view_invalid() {
    let t = LendingTest::new().dual_source_two_asset();
    let usdc = t.resolve_asset("USDC");

    t.env.as_contract(&t.mock_reflector, || {
        let key = test_harness::mock_reflector::MockKey::Spot(usdc.clone());
        t.env.storage().temporary().remove(&key);
    });

    let assets = vec![&t.env, hub_asset(usdc)];
    let row = t
        .ctrl_client()
        .get_market_indexes_detailed(&assets)
        .get(0)
        .unwrap();
    assert!(!row.valid);
    assert!(row.deviation);
}
