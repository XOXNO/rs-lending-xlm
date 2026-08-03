use soroban_sdk::String;
use test_harness::oracle::redstone::{
    anchor_market_with_redstone, anchor_market_with_redstone_feed, redstone_counters,
    register_redstone_adapter,
};
use test_harness::{
    apply_flash_fee, assert_contract_error, build_aggregator_swap, errors, eth_preset,
    redstone_single_config, usd, usdc_preset, wbtc_preset, xlm_preset, LendingTest, ALICE, BOB,
    DEFAULT_TOLERANCE,
};

#[test]
fn test_borrow_tx_fires_one_bulk_redstone_call() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);

    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");

    t.supply(BOB, "ETH", 100.0);

    t.supply(ALICE, "USDC", 10_000.0);

    let rs = redstone_counters(&t, &redstone);
    let single_before = rs.single_calls();
    let bulk_before = rs.bulk_calls();

    t.borrow(ALICE, "ETH", 1.0);

    let rs = redstone_counters(&t, &redstone);
    assert_eq!(
        rs.bulk_calls() - bulk_before,
        1,
        "borrow tx must bulk-fetch RedStone feeds exactly once across all prefetch sites"
    );
    assert_eq!(
        rs.single_calls() - single_before,
        0,
        "no per-feed RedStone calls when bulk prefetch covers the set"
    );
}

#[test]
fn test_multi_asset_supply_fires_zero_redstone_calls() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");

    let rs = redstone_counters(&t, &redstone);
    let bulk_before = rs.bulk_calls();
    let single_before = rs.single_calls();

    t.supply_bulk(ALICE, &[("USDC", 100.0), ("ETH", 1.0)]);

    let rs = redstone_counters(&t, &redstone);
    assert_eq!(
        rs.bulk_calls() - bulk_before,
        0,
        "supply must not bulk-fetch RedStone feeds"
    );
    assert_eq!(
        rs.single_calls() - single_before,
        0,
        "supply must not single-fetch RedStone feeds"
    );
}

#[test]
fn test_bulk_failure_falls_back_to_per_feed_reads() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");

    let eth_feed_id = String::from_str(&t.env, "ETH");
    t.env.as_contract(&redstone, || {
        let key = test_harness::mock_redstone::MockKey::PriceData(eth_feed_id);
        t.env.storage().temporary().remove(&key);
    });

    let rs = redstone_counters(&t, &redstone);
    let single_before_setup = rs.single_calls();
    let bulk_before_setup = rs.bulk_calls();

    t.supply(BOB, "ETH", 100.0);

    t.supply(ALICE, "USDC", 10_000.0);

    let rs = redstone_counters(&t, &redstone);
    assert_eq!(
        rs.single_calls(),
        single_before_setup,
        "supply must not single-fetch RedStone feeds"
    );
    assert_eq!(
        rs.bulk_calls(),
        bulk_before_setup,
        "supply must not bulk-fetch RedStone feeds"
    );

    let result = t.try_borrow(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::UNSAFE_PRICE);
}

#[test]
fn test_bulk_length_mismatch_falls_back_to_per_feed_reads() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");
    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10_000.0);

    let rs = redstone_counters(&t, &redstone);
    rs.set_bulk_truncate(&true);
    let bulk_before = rs.bulk_calls();
    let single_before = rs.single_calls();

    t.borrow(ALICE, "ETH", 1.0);

    let rs = redstone_counters(&t, &redstone);
    assert_eq!(rs.bulk_calls() - bulk_before, 1);
    assert_eq!(rs.single_calls() - single_before, 2);
}

#[test]
fn test_prefetched_prices_resolve_to_expected_values() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");

    t.supply(BOB, "ETH", 100.0);

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    t.assert_healthy(ALICE);

    let total_coll = t.total_collateral(ALICE);
    let total_debt = t.total_debt(ALICE);
    assert!(
        total_coll > 9_900.0 && total_coll < 10_100.0,
        "collateral must be near $10 000 (got {})",
        total_coll
    );
    assert!(
        total_debt > 1_980.0 && total_debt < 2_020.0,
        "debt must be near $2 000 (got {})",
        total_debt
    );
}

#[test]
fn test_withdraw_with_debt_uses_one_bulk_redstone_call() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);

    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");

    t.supply(BOB, "ETH", 100.0);

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let rs = redstone_counters(&t, &redstone);
    let bulk_before = rs.bulk_calls();
    let single_before = rs.single_calls();

    t.withdraw(ALICE, "USDC", 100.0);

    let rs = redstone_counters(&t, &redstone);
    assert_eq!(
        rs.bulk_calls() - bulk_before,
        1,
        "withdraw with debt must bulk-fetch RedStone feeds once"
    );
    assert_eq!(
        rs.single_calls() - single_before,
        0,
        "no per-feed RedStone calls when the entrypoint prefetch covers the set"
    );
}

#[test]
fn test_full_repay_fires_zero_redstone_calls() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    let redstone = register_redstone_adapter(
        &t,
        &[("USDC", usd(1)), ("ETH", usd(2000)), ("WBTC", usd(60_000))],
    );
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");
    anchor_market_with_redstone(&t, &redstone, "WBTC");

    t.supply(BOB, "ETH", 100.0);
    t.supply(BOB, "WBTC", 10.0);

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.borrow(ALICE, "WBTC", 0.1);

    let rs = redstone_counters(&t, &redstone);
    let bulk_before = rs.bulk_calls();
    let single_before = rs.single_calls();

    t.repay_bulk(ALICE, &[("ETH", 2.0), ("WBTC", 0.5)]);

    let rs = redstone_counters(&t, &redstone);
    assert_eq!(
        rs.bulk_calls() - bulk_before,
        0,
        "full repay must fire zero bulk RedStone calls"
    );
    assert_eq!(
        rs.single_calls() - single_before,
        0,
        "full repay must fire zero single RedStone calls"
    );
}

#[test]
fn test_no_debt_withdraw_fires_zero_redstone_calls() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");

    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 1.0);

    let rs = redstone_counters(&t, &redstone);
    let bulk_before = rs.bulk_calls();
    let single_before = rs.single_calls();

    t.withdraw(ALICE, "USDC", 100.0);

    let rs = redstone_counters(&t, &redstone);
    assert_eq!(
        rs.bulk_calls() - bulk_before,
        0,
        "no-debt withdraw must fire zero bulk RedStone calls"
    );
    assert_eq!(
        rs.single_calls() - single_before,
        0,
        "no-debt withdraw must fire zero single RedStone calls"
    );
}

#[test]
fn test_no_debt_bulk_full_close_fires_zero_redstone_calls() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");

    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 1.0);

    let rs = redstone_counters(&t, &redstone);
    let bulk_before = rs.bulk_calls();
    let single_before = rs.single_calls();

    t.withdraw_bulk(ALICE, &[("USDC", 0.0), ("ETH", 0.0)]);

    let rs = redstone_counters(&t, &redstone);
    assert_eq!(
        rs.bulk_calls() - bulk_before,
        0,
        "no-debt bulk full close must fire zero bulk RedStone calls"
    );
    assert_eq!(
        rs.single_calls() - single_before,
        0,
        "no-debt bulk full close must fire zero single RedStone calls"
    );
    assert_eq!(t.get_active_accounts(ALICE).len(), 0);
}

#[test]
fn test_two_adapters_bulk_once_each() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_market(xlm_preset())
        .build();

    let adapter_a = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);
    let adapter_b = register_redstone_adapter(&t, &[("WBTC", usd(60_000)), ("XLM", usd(1) / 10)]);

    anchor_market_with_redstone(&t, &adapter_a, "USDC");
    anchor_market_with_redstone(&t, &adapter_a, "ETH");
    anchor_market_with_redstone(&t, &adapter_b, "WBTC");
    anchor_market_with_redstone(&t, &adapter_b, "XLM");

    t.supply(BOB, "ETH", 100.0);
    t.supply(BOB, "WBTC", 10.0);
    t.supply(BOB, "XLM", 1_000_000.0);

    t.supply(ALICE, "USDC", 1_000_000.0);
    t.borrow(ALICE, "ETH", 0.1);
    t.borrow(ALICE, "WBTC", 0.001);
    t.borrow(ALICE, "XLM", 100.0);

    let rs_a = redstone_counters(&t, &adapter_a);
    let rs_b = redstone_counters(&t, &adapter_b);
    let bulk_a_before = rs_a.bulk_calls();
    let single_a_before = rs_a.single_calls();
    let bulk_b_before = rs_b.bulk_calls();
    let single_b_before = rs_b.single_calls();

    t.borrow(ALICE, "ETH", 0.01);

    let rs_a = redstone_counters(&t, &adapter_a);
    let rs_b = redstone_counters(&t, &adapter_b);

    assert_eq!(
        rs_a.bulk_calls() - bulk_a_before,
        1,
        "adapter A must fire exactly one bulk call when it has 2 feeds in the position set"
    );
    assert_eq!(
        rs_b.bulk_calls() - bulk_b_before,
        1,
        "adapter B must fire exactly one bulk call when it has 2 feeds in the position set"
    );
    assert_eq!(
        rs_a.single_calls() - single_a_before,
        0,
        "no single calls on adapter A when bulk covers both feeds"
    );
    assert_eq!(
        rs_b.single_calls() - single_b_before,
        0,
        "no single calls on adapter B when bulk covers both feeds"
    );
}

#[test]
fn test_prefetch_skips_unlisted_asset_without_panic() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);

    anchor_market_with_redstone_feed(&t, &redstone, "USDC", "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");

    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.assert_healthy(ALICE);
}

#[test]
fn test_shared_feed_two_assets_single_redstone_call() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let redstone = register_redstone_adapter(&t, &[("SHARED", usd(1))]);
    let feed_id = String::from_str(&t.env, "SHARED");

    let usdc_cfg = redstone_single_config(
        &t.env,
        &redstone,
        &feed_id,
        usd(1),
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    let eth_cfg = redstone_single_config(
        &t.env,
        &redstone,
        &feed_id,
        usd(1),
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    t.configure_market_oracle(&t.resolve_asset("USDC"), &usdc_cfg);
    t.configure_market_oracle(&t.resolve_asset("ETH"), &eth_cfg);

    t.supply(BOB, "ETH", 10_000.0);
    t.supply(ALICE, "USDC", 1_000_000.0);

    let rs = redstone_counters(&t, &redstone);
    let single_before = rs.single_calls();
    let bulk_before = rs.bulk_calls();

    t.borrow(ALICE, "ETH", 100.0);

    let rs = redstone_counters(&t, &redstone);
    assert_eq!(
        rs.bulk_calls() - bulk_before,
        0,
        "single shared feed is below MIN_BULK_FEEDS — no bulk call expected"
    );
    assert_eq!(
        rs.single_calls() - single_before,
        1,
        "lazy-warm fix: first single read fills the cache; second consumer is a hit — exactly 1 call"
    );
}

#[test]
fn test_liquidation_fires_one_bulk_redstone_call() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");

    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.assert_healthy(ALICE);

    let rs_client = redstone_counters(&t, &redstone);
    rs_client.set_price(&String::from_str(&t.env, "ETH"), &usd(4000));
    t.set_price("ETH", usd(4000));

    t.assert_liquidatable(ALICE);

    let rs = redstone_counters(&t, &redstone);
    let bulk_before = rs.bulk_calls();
    let single_before = rs.single_calls();

    t.liquidate("liquidator", ALICE, "ETH", 1.0);

    let rs = redstone_counters(&t, &redstone);
    assert_eq!(
        rs.bulk_calls() - bulk_before,
        1,
        "liquidation HF check must bulk-fetch RedStone feeds exactly once"
    );
    assert_eq!(
        rs.single_calls() - single_before,
        0,
        "no per-feed calls when bulk prefetch covers the liquidation position set"
    );
}

#[test]
fn test_redstone_primary_markets_fire_one_bulk() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);
    let usdc_feed = String::from_str(&t.env, "USDC");
    let eth_feed = String::from_str(&t.env, "ETH");

    let usdc_cfg = redstone_single_config(
        &t.env,
        &redstone,
        &usdc_feed,
        usd(1),
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    let eth_cfg = redstone_single_config(
        &t.env,
        &redstone,
        &eth_feed,
        usd(2_000),
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    t.configure_market_oracle(&t.resolve_asset("USDC"), &usdc_cfg);
    t.configure_market_oracle(&t.resolve_asset("ETH"), &eth_cfg);

    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10_000.0);

    let rs = redstone_counters(&t, &redstone);
    let bulk_before = rs.bulk_calls();
    let single_before = rs.single_calls();

    t.borrow(ALICE, "ETH", 1.0);

    let rs = redstone_counters(&t, &redstone);
    assert_eq!(
        rs.bulk_calls() - bulk_before,
        1,
        "two primary-RedStone markets must trigger one bulk call"
    );
    assert_eq!(
        rs.single_calls() - single_before,
        0,
        "no per-feed calls when bulk prefetch covers both primary feeds"
    );

    let total_coll = t.total_collateral(ALICE);
    let total_debt = t.total_debt(ALICE);
    assert!(
        total_coll > 9_900.0 && total_coll < 10_100.0,
        "primary-RedStone collateral must resolve to mock price (got {})",
        total_coll
    );
    assert!(
        total_debt > 1_980.0 && total_debt < 2_020.0,
        "primary-RedStone debt must resolve to mock price (got {})",
        total_debt
    );
}

#[test]
fn test_same_asset_supplied_and_borrowed_one_call() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1))]);
    anchor_market_with_redstone(&t, &redstone, "USDC");

    t.supply(BOB, "USDC", 100_000.0);
    t.supply(ALICE, "USDC", 10_000.0);

    let rs = redstone_counters(&t, &redstone);
    let single_before = rs.single_calls();
    let bulk_before = rs.bulk_calls();

    t.borrow(ALICE, "USDC", 100.0);

    let rs = redstone_counters(&t, &redstone);
    assert_eq!(
        rs.bulk_calls() - bulk_before,
        0,
        "1 feed < MIN_BULK_FEEDS: no bulk call"
    );
    assert_eq!(
        rs.single_calls() - single_before,
        1,
        "single shared (supply+borrow) feed: exactly 1 RedStone call"
    );
}

#[test]
fn test_mixed_adapter_groups() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    let adapter_a = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);
    let adapter_b = register_redstone_adapter(&t, &[("WBTC", usd(60_000))]);

    anchor_market_with_redstone(&t, &adapter_a, "USDC");
    anchor_market_with_redstone(&t, &adapter_a, "ETH");
    anchor_market_with_redstone(&t, &adapter_b, "WBTC");

    t.supply(BOB, "ETH", 100.0);
    t.supply(BOB, "WBTC", 10.0);

    t.supply(ALICE, "USDC", 1_000_000.0);
    t.borrow(ALICE, "ETH", 0.1);

    let rs_a = redstone_counters(&t, &adapter_a);
    let rs_b = redstone_counters(&t, &adapter_b);
    let bulk_a_before = rs_a.bulk_calls();
    let single_a_before = rs_a.single_calls();
    let bulk_b_before = rs_b.bulk_calls();
    let single_b_before = rs_b.single_calls();

    t.borrow(ALICE, "WBTC", 0.001);

    let rs_a = redstone_counters(&t, &adapter_a);
    let rs_b = redstone_counters(&t, &adapter_b);

    assert_eq!(
        rs_a.bulk_calls() - bulk_a_before,
        1,
        "adapter A (2 feeds) must fire exactly one bulk call"
    );
    assert_eq!(
        rs_a.single_calls() - single_a_before,
        0,
        "adapter A: no single calls when bulk covers both feeds"
    );

    assert_eq!(
        rs_b.bulk_calls() - bulk_b_before,
        0,
        "adapter B (1 feed) must fire zero bulk calls"
    );
    assert_eq!(
        rs_b.single_calls() - single_b_before,
        1,
        "adapter B: exactly one single call for the sole feed"
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn test_committed_bulk_failure_reverts_on_missing_anchor() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");

    t.supply_bulk(ALICE, &[("USDC", 100.0), ("ETH", 1.0)]);

    t.env.as_contract(&redstone, || {
        let key = test_harness::mock_redstone::MockKey::PriceData(String::from_str(&t.env, "ETH"));
        t.env.storage().temporary().remove(&key);
    });

    let _ = t.total_collateral(ALICE);
}

#[test]
fn test_stale_payload_through_bulk_is_still_rejected() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");

    let stale_ms = t.env.ledger().timestamp().saturating_sub(950) * 1000;
    let rs_client = redstone_counters(&t, &redstone);
    rs_client.set_price_data(
        &String::from_str(&t.env, "ETH"),
        &usd(2000),
        &stale_ms,
        &stale_ms,
    );

    t.supply(BOB, "ETH", 100.0);
    t.supply_bulk(ALICE, &[("USDC", 10_000.0), ("ETH", 1.0)]);

    let result = t.try_borrow(ALICE, "ETH", 0.001);
    assert_contract_error(result, errors::OracleError::PriceFeedStale as u32);
}

#[test]
fn test_disabled_market_panics_same_through_prefetch() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");

    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    t.price_agg_client()
        .remove_oracle(&controller::types::PriceKey::Token(t.resolve_asset("ETH")));

    let result = t.try_borrow(ALICE, "ETH", 0.001);
    assert_contract_error(result, errors::OracleError::OracleNotConfigured as u32);
}

#[test]
fn test_multiply_fires_one_bulk_redstone_call() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");

    t.fund_router("USDC", 3_000.0);
    let steps = build_aggregator_swap(
        &t,
        "ETH",
        "USDC",
        apply_flash_fee(10_000_000),
        30_000_000_000,
    );

    let rs = redstone_counters(&t, &redstone);
    let bulk_before = rs.bulk_calls();
    let single_before = rs.single_calls();

    t.multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        controller::types::PositionMode::Multiply,
        &steps,
    );

    let rs = redstone_counters(&t, &redstone);
    assert_eq!(
        rs.bulk_calls() - bulk_before,
        1,
        "multiply must bulk-fetch RedStone feeds exactly once"
    );
    assert_eq!(
        rs.single_calls() - single_before,
        0,
        "no per-feed RedStone calls when bulk prefetch covers the multiply set"
    );
}

#[test]
fn test_aggregate_views_fire_one_bulk_redstone_call() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    let redstone = register_redstone_adapter(
        &t,
        &[("USDC", usd(1)), ("ETH", usd(2000)), ("WBTC", usd(60_000))],
    );
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");
    anchor_market_with_redstone(&t, &redstone, "WBTC");

    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 1.0);
    t.supply(ALICE, "WBTC", 0.1);

    let rs = redstone_counters(&t, &redstone);
    let bulk_before = rs.bulk_calls();
    let single_before = rs.single_calls();

    t.total_collateral(ALICE);

    let rs = redstone_counters(&t, &redstone);
    assert_eq!(
        rs.bulk_calls() - bulk_before,
        1,
        "total_collateral_in_usd over 3 RedStone markets must fire one bulk call"
    );
    assert_eq!(
        rs.single_calls() - single_before,
        0,
        "no per-feed calls when bulk prefetch covers all supply positions"
    );

    t.supply(BOB, "USDC", 100_000.0);
    t.supply(BOB, "ETH", 100.0);
    t.borrow(ALICE, "USDC", 100.0);
    t.borrow(ALICE, "ETH", 0.01);

    let rs = redstone_counters(&t, &redstone);
    let bulk_before = rs.bulk_calls();
    let single_before = rs.single_calls();

    t.total_debt(ALICE);

    let rs = redstone_counters(&t, &redstone);
    assert_eq!(
        rs.bulk_calls() - bulk_before,
        1,
        "total_borrow_in_usd over 2 RedStone debt positions must fire one bulk call"
    );
    assert_eq!(
        rs.single_calls() - single_before,
        0,
        "no per-feed calls when bulk prefetch covers all debt positions"
    );
}
