//! INV-ORACLE-04 on the LIVE path.
//!
//! `docs/reference/invariants.md` records a verification gap: the Certora
//! rules `timestamp_at_future_skew_boundary_is_allowed` and
//! `timestamp_beyond_future_skew_reverts` exercise `check_not_future_at`,
//! which no contract calls. Production uses `is_future_at`, which DROPS the
//! leg instead of panicking. These tests cover the live helper end to end:
//! the boundary it enforces, and that a dropped leg fails closed rather than
//! degrading a dual source to a single leg (INV-ORACLE-02).

use common::oracle::observation::MAX_FUTURE_SKEW_SECONDS;
use soroban_sdk::{Address, String};
use test_harness::oracle::redstone::register_redstone_adapter;
use test_harness::{hub_asset, usd, usdc_preset, LendingTest, ALICE, BOB, DEFAULT_TOLERANCE};

/// Stamps both the package and write timestamps of `feed_id` at
/// `now + offset` seconds. `from_multi_feed` gates on both.
fn stamp_feed_at_offset(t: &LendingTest, redstone: &Address, feed_id: &String, offset: u64) {
    let client = test_harness::mock_redstone::MockRedStonePriceFeedClient::new(&t.env, redstone);
    let ts_ms = (t.env.ledger().timestamp() + offset) * 1_000;
    client.set_price_data(feed_id, &usd(1), &ts_ms, &ts_ms);
}

fn single_source_usdc(t: &LendingTest, redstone: &Address, feed_id: &String) {
    let asset = t.resolve_asset("USDC");
    let cfg = test_harness::redstone_single_config(
        &t.env,
        redstone,
        feed_id,
        usd(1),
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    t.configure_market_oracle(&asset, &cfg);
}

/// A feed stamped at exactly `now + MAX_FUTURE_SKEW_SECONDS` is a valid
/// observation. This is the boundary the Certora rule proves on the helper the
/// contracts never call; here it is proved on the live path. The borrow is
/// what forces valuation -- a bare supply does not read a price.
#[test]
fn future_skew_boundary_is_accepted_on_the_live_path() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_asset("USDC");
    let feed_id = String::from_str(&t.env, "USDC");
    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1))]);
    single_source_usdc(&t, &redstone, &feed_id);

    t.supply(BOB, "USDC", 100_000.0);
    t.supply(ALICE, "USDC", 10_000.0);

    stamp_feed_at_offset(&t, &redstone, &feed_id, MAX_FUTURE_SKEW_SECONDS);

    let assets = soroban_sdk::Vec::from_array(&t.env, [hub_asset(asset)]);
    let row = t
        .ctrl_client()
        .get_market_indexes_detailed(&assets)
        .get(0)
        .unwrap();
    assert!(
        row.valid,
        "a feed exactly at the skew bound must stay valid"
    );
    assert_eq!(row.price_wad, usd(1));

    t.borrow(ALICE, "USDC", 100.0);
}

/// One second past the bound the leg is dropped. With a single source that
/// leaves `Legs::Empty`, so the valuation-dependent mutation must revert
/// (INV-ORACLE-01) rather than price the asset at zero.
#[test]
#[should_panic(expected = "Error(Contract, #210)")]
fn one_second_past_future_skew_fails_closed_on_the_live_path() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    let feed_id = String::from_str(&t.env, "USDC");
    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1))]);
    single_source_usdc(&t, &redstone, &feed_id);

    t.supply(BOB, "USDC", 100_000.0);
    t.supply(ALICE, "USDC", 10_000.0);

    stamp_feed_at_offset(&t, &redstone, &feed_id, MAX_FUTURE_SKEW_SECONDS + 1);
    t.borrow(ALICE, "USDC", 100.0);
}

/// The quiet-degradation case the dropping helper makes possible: a dual
/// source whose anchor leg is future-dated must NOT fall back to the healthy
/// primary leg. The view must report the outcome unusable.
#[test]
fn future_dated_anchor_does_not_degrade_to_a_single_leg() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_asset("USDC");
    let feed_id = String::from_str(&t.env, "USDC");
    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1))]);

    let cfg = test_harness::reflector_primary_redstone_anchor_config(
        &t.env,
        &t.mock_reflector,
        &asset,
        &redstone,
        &feed_id,
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    t.configure_market_oracle(&asset, &cfg);

    stamp_feed_at_offset(&t, &redstone, &feed_id, MAX_FUTURE_SKEW_SECONDS + 1);

    let assets = soroban_sdk::Vec::from_array(&t.env, [hub_asset(asset)]);
    let row = t
        .ctrl_client()
        .get_market_indexes_detailed(&assets)
        .get(0)
        .unwrap();

    assert!(!row.valid, "a dropped anchor leg must not report valid");
    assert!(row.deviation, "a missing dual-source leg is a deviation");
    assert_eq!(
        row.anchor_price_wad, 0,
        "the future-dated leg must contribute nothing"
    );
    assert_eq!(
        row.price_wad, 0,
        "no single-leg fallback price may be published"
    );
}
