//! Xoxno provider markets: RedStone wire shape with listing-time decimal probing.

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

/// A non-default adapter width works when the config declares it.
///
/// v1 read `decimals()` off the adapter on every listing. The composable model
/// carries the width as config data (so reads stay cheap) and verifies that
/// declaration against the adapter once, at configure time. Either way an
/// operator cannot end up 10x mis-scaled — see the companion test below for the
/// half that v1 could not express at all.
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
}

/// The other half: a config declaring the wrong width against a live adapter is
/// rejected (`InvalidOracleDecimals`, #221).
///
/// This is the case that makes carrying decimals as config data safe. Left
/// unchecked, declaring 8 against a 9-decimal adapter would read a fresh,
/// in-band-looking price that is wrong by 10x — and nothing downstream could
/// catch it, because the proposer sets the sanity band too.
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

    // Declares the RedStone default of 8 against a 9-decimal adapter.
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
    // Full path against the real `xoxno-oracle` contract, no mock:
    // 2-of-3 signer submissions, listing-time SEP-40 `decimals()` probe, and
    // a priced supply through the controller.
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
    // Both feeds agree at $1, so the in-band blend is the midpoint $1.
    assert_eq!(view.price_wad, usd(1));
}
