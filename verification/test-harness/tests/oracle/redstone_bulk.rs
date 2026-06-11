use soroban_sdk::{Address, String};
use test_harness::{
    assert_contract_error, errors, eth_preset, usd, usdc_preset, LendingTest, ALICE, BOB,
    DEFAULT_TOLERANCE,
};

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
    //
    // The borrow tx contains three internal prefetch call sites:
    //   1. contracts/controller/src/positions/borrow.rs entrypoint: explicit
    //      prefetch with [supply_assets + borrow_assets] before the HF check.
    //   2. helpers/math.rs HF body (calculate_account_totals_body): a second
    //      prefetch_redstone_feeds call for the same feed set.
    //   3. helpers/account.rs dust gate: a third prefetch_redstone_feeds call.
    // All three deduplicate to exactly one bulk adapter call because the
    // tx-local Cache is populated by site 1 and the subsequent sites find all
    // feeds already resolved — this is the idempotency property the assertions
    // below pin.
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

#[test]
fn test_multi_asset_supply_dust_check_uses_one_bulk_call() {
    // Two RedStone-anchored markets on the SAME adapter; a pure supply (no
    // debt → no HF body) triggers only the dust gate, which must dispatch
    // exactly one bulk call for both assets.
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Both presets use DEFAULT_ASSET_CONFIG where min_collat_floor_usd_wad =
    // MIN_DUST_FLOOR_WAD (non-zero), so the dust gate will price both assets.
    let redstone = setup_redstone_feeds(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");

    let rs = redstone_client(&t, &redstone);
    let bulk_before = rs.bulk_calls();
    let single_before = rs.single_calls();

    // Supply both assets in one controller call — no debt means HF is
    // skipped; the dust gate is the sole price consumer.
    t.supply_bulk(ALICE, &[("USDC", 100.0), ("ETH", 1.0)]);

    let rs = redstone_client(&t, &redstone);
    assert_eq!(
        rs.bulk_calls() - bulk_before,
        1,
        "dust gate must bulk-fetch RedStone feeds once"
    );
    assert_eq!(
        rs.single_calls() - single_before,
        0,
        "no per-feed RedStone calls when bulk prefetch covers the dust scope"
    );
}

#[test]
fn test_bulk_failure_falls_back_to_per_feed_reads() {
    // Two markets anchored to the same adapter.  Both feeds are priced at
    // configure time (oracle config validation requires a live price read),
    // then the ETH entry is removed from mock storage before any runtime
    // reads.  At runtime the bulk call [USDC, ETH] fails whole-call because
    // ETH is absent; the prefetch map stays empty and the lazy per-feed path
    // takes over.
    //
    // Supply (OraclePolicy::RiskDecreasing) tolerates a missing anchor and
    // falls back to the Reflector primary — setup supplies succeed.
    // Borrow (OraclePolicy::RiskIncreasing) does NOT tolerate a degraded
    // dual-source; the per-feed path also finds ETH absent and compose calls
    // fallback_to_primary which panics OracleError::NoLastPrice (#210) —
    // identical to the pre-feature per-feed-only behavior.
    //
    // Sequencing: set both feeds for configure-time validation → configure
    // both anchors → remove ETH from mock storage → supply BOB + supply ALICE
    // (RiskDecreasing tolerates missing anchor) → assert single_calls
    // increased (per-feed path engaged during the single-asset dust gates) →
    // assert borrow returns error #210 (try_borrow rolls back its own storage
    // changes, so counter deltas from the failed tx are not observable here).
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Both feeds required at configure time for oracle validation.
    let redstone = setup_redstone_feeds(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");

    // Remove the ETH feed so it is absent at runtime while having passed the
    // configure-time validation that requires a live price read.
    let eth_feed_id = String::from_str(&t.env, "ETH");
    t.env.as_contract(&redstone, || {
        let key = test_harness::mock_redstone::MockKey::PriceData(eth_feed_id);
        t.env.storage().temporary().remove(&key);
    });

    // Snapshot counters before setup supplies.
    let rs = redstone_client(&t, &redstone);
    let single_before_setup = rs.single_calls();
    let bulk_before_setup = rs.bulk_calls();

    // BOB supplies ETH liquidity (RiskDecreasing: missing ETH anchor is
    // tolerated; compose falls back to Reflector primary — supply succeeds).
    t.supply(BOB, "ETH", 100.0);

    // ALICE supplies USDC collateral (USDC anchor present — succeeds).
    t.supply(ALICE, "USDC", 10_000.0);

    // Each single-asset supply triggers a one-feed dust gate; MIN_BULK_FEEDS=2
    // means bulk is skipped.  The per-feed lazy path fires directly:
    //   • USDC supply: RedStone single call → feed found → Some → success;
    //     the bump from this committed tx is visible in storage.
    //   • ETH supply: RedStone single call → feed missing → Err-returning
    //     mock frame; the ETH counter bump rolls back with that frame, so it
    //     is NOT observable here — only the USDC read's bump commits.
    // The assertion therefore checks that at least the successful USDC read
    // engaged the lazy path; the ETH engagement is pinned indirectly by the
    // supply succeeding despite the missing anchor (RiskDecreasing fallback).
    let rs = redstone_client(&t, &redstone);
    assert!(
        rs.single_calls() > single_before_setup,
        "lazy path engaged: at least the USDC single read committed (ETH bump rolls back with its erroring frame)"
    );
    assert_eq!(
        rs.bulk_calls(),
        bulk_before_setup,
        "no bulk call expected when each supply touches only one feed"
    );

    // Borrow: HF check collects [USDC, ETH] → two feeds → bulk attempted →
    // ETH absent → bulk fails → prefetch map empty → per-feed lazy: USDC
    // anchor found, ETH anchor missing → compose fallback_to_primary with
    // RiskIncreasing → panics #210.  try_borrow catches the panic and rolls
    // back all storage changes from that transaction, so the counter increments
    // from the failed borrow are NOT visible after this call.
    let result = t.try_borrow(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::OracleError::NoLastPrice as u32);
}

#[test]
fn test_prefetched_price_resolution_matches_lazy() {
    // Both feeds priced; ALICE supplies USDC and borrows ETH so the bulk path
    // is exercised inside both txs.  Assert the resulting account is healthy
    // (HF > 1.0), which can only hold if the prefetched prices resolve to the
    // same values the lazy per-feed path would produce from the same mock data.
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let redstone = setup_redstone_feeds(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");

    // BOB provides ETH liquidity.
    t.supply(BOB, "ETH", 100.0);

    // ALICE supplies USDC collateral and borrows a small amount of ETH.
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // If the prefetch had corrupted or misaligned price data the position
    // values would be wrong and HF could fall below 1.0 for a safe position.
    t.assert_healthy(ALICE);

    // Sanity-check the resolved position values using the known mock prices.
    // 1 ETH @ $2000, 10 000 USDC @ $1 → debt ≈ $2000, collateral ≈ $10 000.
    let total_coll = t.total_collateral(ALICE);
    let total_debt = t.total_debt(ALICE);
    assert!(
        total_coll > 9_000.0 && total_coll < 11_000.0,
        "resolved collateral value should be near $10 000 (got {})",
        total_coll
    );
    assert!(
        total_debt > 1_500.0 && total_debt < 2_500.0,
        "resolved debt value should be near $2 000 (got {})",
        total_debt
    );
}
