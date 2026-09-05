use soroban_sdk::String;
use test_harness::oracle::redstone::register_redstone_adapter;
use test_harness::oracle::xoxno::register_xoxno_adapter;
use test_harness::{hub_asset, usd, usdc_preset, LendingTest, ALICE, DEFAULT_TOLERANCE};

#[test]
fn test_xoxno_single_source_market_works() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_asset("USDC");
    let feed_id = String::from_str(&t.env, "USDC");
    let adapter = register_redstone_adapter(&t, &[("USDC", usd(1))]);

    let cfg = test_harness::xoxno_single_config(
        &t.env,
        &adapter,
        &feed_id,
        usd(1),
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    t.configure_market_oracle(&asset, &cfg);

    t.supply(ALICE, "USDC", 1_000.0);
    t.assert_supply_near(ALICE, "USDC", 1_000.0, 1.0);
}

#[test]
fn test_xoxno_listing_accepts_a_declared_non_default_adapter_width() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_asset("USDC");
    let feed_id = String::from_str(&t.env, "USDC");
    let adapter = register_redstone_adapter(&t, &[]);

    let client = test_harness::mock_redstone::MockRedStonePriceFeedClient::new(&t.env, &adapter);
    client.set_decimals(&9);
    client.set_price(&feed_id, &usd(1));

    let cfg = test_harness::xoxno_single_config_with_decimals(
        &t.env,
        &adapter,
        &feed_id,
        9,
        usd(1),
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    t.configure_market_oracle(&asset, &cfg);

    let assets = soroban_sdk::Vec::from_array(&t.env, [hub_asset(asset)]);
    let view = t
        .ctrl_client()
        .get_market_indexes_detailed(&assets)
        .get(0)
        .unwrap();
    assert_eq!(view.price_wad, usd(1));
    assert!(view.valid, "a declared 9-decimal adapter must price valid");
    assert!(!view.stale);
    assert!(!view.deviation);
    assert_eq!(view.error_code, None);
}

#[test]
#[should_panic(expected = "Error(Contract, #221)")]
fn test_xoxno_listing_rejects_a_width_the_adapter_contradicts() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_asset("USDC");
    let feed_id = String::from_str(&t.env, "USDC");
    let adapter = register_redstone_adapter(&t, &[]);

    let client = test_harness::mock_redstone::MockRedStonePriceFeedClient::new(&t.env, &adapter);
    client.set_decimals(&9);
    client.set_price(&feed_id, &usd(1));

    let cfg = test_harness::xoxno_single_config(
        &t.env,
        &adapter,
        &feed_id,
        usd(1),
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    t.configure_market_oracle(&asset, &cfg);
}

#[test]
fn test_real_adapter_single_source_market_end_to_end() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_asset("USDC");
    let feed_id = String::from_str(&t.env, "USDC");
    let (adapter, _signers) = register_xoxno_adapter(&t, &[("USDC", usd(1))], 3, 2);

    let cfg = test_harness::xoxno_single_config(
        &t.env,
        &adapter,
        &feed_id,
        usd(1),
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    t.configure_market_oracle(&asset, &cfg);

    let assets = soroban_sdk::Vec::from_array(&t.env, [hub_asset(asset)]);
    let view = t
        .ctrl_client()
        .get_market_indexes_detailed(&assets)
        .get(0)
        .unwrap();
    assert_eq!(view.price_wad, usd(1));
    assert!(view.valid, "a quorum-signed adapter price must read valid");
    assert!(!view.stale);
    assert_eq!(view.error_code, None);

    t.supply(ALICE, "USDC", 1_000.0);
    t.assert_supply_near(ALICE, "USDC", 1_000.0, 1.0);
}

#[test]
fn test_reflector_primary_xoxno_anchor_market_works() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_asset("USDC");
    let feed_id = String::from_str(&t.env, "USDC");
    let adapter = register_redstone_adapter(&t, &[("USDC", usd(1))]);

    let cfg = test_harness::reflector_primary_xoxno_anchor_config(
        &t.env,
        &t.mock_reflector,
        &asset,
        &adapter,
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

    assert_eq!(view.price_wad, usd(1));
    assert!(view.valid, "both legs fresh and in band must read valid");
    assert!(!view.stale);
    assert!(!view.deviation);
    assert_eq!(view.error_code, None);
}
