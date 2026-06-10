use soroban_sdk::{Address, String};
use test_harness::{eth_preset, usd, usdc_preset, LendingTest, ALICE, BOB, DEFAULT_TOLERANCE};

/// One mock adapter serving multiple feeds, registered + priced.
fn setup_redstone_feeds(t: &LendingTest, feeds: &[(&str, i128)]) -> Address {
    let redstone = t
        .env
        .register(test_harness::mock_redstone::MockRedStonePriceFeed, ());
    let client =
        test_harness::mock_redstone::MockRedStonePriceFeedClient::new(&t.env, &redstone);
    for (feed, price_wad) in feeds {
        client.set_price(&String::from_str(&t.env, feed), price_wad);
    }
    redstone
}

/// Reflector primary + RedStone anchor on `symbol`, same shape as prod config.
fn anchor_market_with_redstone(t: &LendingTest, redstone: &Address, symbol: &str) {
    let asset = t.resolve_asset(symbol);
    let feed_id = String::from_str(&t.env, symbol);
    let cfg = test_harness::reflector_primary_redstone_anchor_config(
        &t.mock_reflector,
        &asset,
        redstone,
        &feed_id,
        DEFAULT_TOLERANCE.first_upper_bps,
        DEFAULT_TOLERANCE.last_upper_bps,
    );
    t.ctrl_client()
        .configure_market_oracle(&t.admin(), &asset, &cfg);
}

fn redstone_client<'a>(
    t: &'a LendingTest,
    redstone: &Address,
) -> test_harness::mock_redstone::MockRedStonePriceFeedClient<'a> {
    test_harness::mock_redstone::MockRedStonePriceFeedClient::new(&t.env, redstone)
}

#[test]
fn test_borrow_hf_uses_one_bulk_redstone_call() {
    // Two RedStone-anchored markets on the SAME adapter; a borrow's HF check
    // prices both feeds and must dispatch exactly one bulk call.
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // One adapter, two feeds (USDC=$1, ETH=$2000).
    let redstone = setup_redstone_feeds(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);

    // Anchor both markets to the single adapter.
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");

    // BOB provides ETH liquidity so ALICE can borrow it.
    t.supply(BOB, "ETH", 100.0);

    // ALICE supplies USDC as collateral.
    t.supply(ALICE, "USDC", 10_000.0);

    // Measure counters BEFORE the borrow (each client call is its own tx).
    let rs = redstone_client(&t, &redstone);
    let single_before = rs.single_calls();
    let bulk_before = rs.bulk_calls();

    // Borrow triggers an HF check that must price BOTH feeds.
    t.borrow(ALICE, "ETH", 1.0);

    // Re-read counters AFTER the borrow.
    let rs = redstone_client(&t, &redstone);
    assert_eq!(
        rs.bulk_calls() - bulk_before,
        1,
        "HF valuation must bulk-fetch RedStone feeds once"
    );
    assert_eq!(
        rs.single_calls() - single_before,
        0,
        "no per-feed RedStone calls when bulk prefetch covers the set"
    );
}
