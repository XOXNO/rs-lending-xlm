use common::constants::WAD;
use soroban_sdk::{Address, String};
use test_harness::{
    assert_contract_error, errors, eth_preset, usd, usdc_preset, wbtc_preset, LendingTest, ALICE,
    BOB, DEFAULT_TOLERANCE,
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

#[test]
fn test_withdraw_with_debt_uses_one_bulk_redstone_call() {
    // Two RedStone-anchored markets on the SAME adapter; a withdraw with an
    // outstanding borrow must price the withdrawn asset AND every remaining
    // position before the LTV/HF check, so the entrypoint prefetch must cover
    // the full position set.
    //
    // Without an entrypoint prefetch the withdrawal loop single-resolves the
    // withdrawn asset BEFORE any prefetch site runs with the full set, leaving
    // the remaining position feeds to be lazily resolved one-at-a-time.
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // One adapter, two feeds (USDC=$1, ETH=$2000).
    let redstone = setup_redstone_feeds(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);

    // Anchor both markets to the single adapter.
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");

    // BOB supplies ETH liquidity so ALICE can borrow it.
    t.supply(BOB, "ETH", 100.0);

    // ALICE supplies USDC as collateral and borrows ETH.
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // Snapshot counters after setup (each operation above is its own tx).
    let rs = redstone_client(&t, &redstone);
    let bulk_before = rs.bulk_calls();
    let single_before = rs.single_calls();

    // ALICE withdraws a small portion of her USDC — small enough to stay healthy.
    // This triggers: withdrawal loop (prices USDC), then require_within_ltv
    // (prices all supply+borrow), then require_healthy_account.  Without an
    // entrypoint prefetch the USDC feed is single-resolved before the bulk
    // opportunity and ETH is resolved lazily too.
    t.withdraw(ALICE, "USDC", 100.0);

    let rs = redstone_client(&t, &redstone);
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
fn test_isolated_multi_asset_repay_uses_one_bulk_redstone_call() {
    // An isolated account with TWO isolation-borrowable debt assets repaying
    // both in one tx.  The isolated path in `process_single_repay` calls
    // `cache.cached_price(asset)` for each repaid asset BEFORE the dust gate
    // runs its own prefetch — so without an entrypoint prefetch the first asset
    // single-resolves its feed before any bulk opportunity.
    //
    // With the fix (`prefetch_redstone_feeds` over plan assets at the
    // `process_repay` entrypoint) both feeds are bulk-fetched once and the
    // per-asset `cached_price` calls find them already resolved.
    let ceiling = 1_000_000 * WAD;
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market_config("USDC", |c| {
            c.is_isolated_asset = true;
            c.isolation_debt_ceiling_usd_wad = ceiling;
        })
        .with_market(eth_preset())
        .with_market_config("ETH", |c| {
            c.isolation_borrow_enabled = true;
        })
        .with_market(wbtc_preset())
        .with_market_config("WBTC", |c| {
            c.isolation_borrow_enabled = true;
        })
        .build();

    // One adapter serves all three feeds.
    let redstone = setup_redstone_feeds(
        &t,
        &[("USDC", usd(1)), ("ETH", usd(2000)), ("WBTC", usd(60_000))],
    );
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");
    anchor_market_with_redstone(&t, &redstone, "WBTC");

    // BOB provides ETH and WBTC liquidity.
    t.supply(BOB, "ETH", 100.0);
    t.supply(BOB, "WBTC", 10.0);

    // ALICE opens an isolated USDC-backed account and borrows both ETH and WBTC.
    t.create_isolated_account(ALICE, "USDC");
    t.supply(ALICE, "USDC", 500_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.borrow(ALICE, "WBTC", 0.1);

    // Snapshot counters before the repay.
    let rs = redstone_client(&t, &redstone);
    let bulk_before = rs.bulk_calls();
    let single_before = rs.single_calls();

    // Repay both debt assets in a single controller call.
    t.repay_bulk(ALICE, &[("ETH", 1.0), ("WBTC", 0.1)]);

    let rs = redstone_client(&t, &redstone);
    assert_eq!(
        rs.bulk_calls() - bulk_before,
        1,
        "isolated multi-asset repay must bulk-fetch RedStone feeds once"
    );
    assert_eq!(
        rs.single_calls() - single_before,
        0,
        "no per-feed RedStone calls when the entrypoint prefetch covers the repay set"
    );
}

// ── Issue 1 regression ────────────────────────────────────────────────────────

#[test]
fn test_non_isolated_full_repay_fires_zero_redstone_calls() {
    // NON-isolated account with two RedStone-anchored debt assets; repaying
    // BOTH IN FULL in one tx.  The non-isolated repay path sets price=Wad::ZERO
    // for each asset, so no pricing happens in the loop.  The dust gate
    // prescreens for open positions and skips fully-closed ones — so zero
    // RedStone reads are needed.
    //
    // Before the fix: the entrypoint `prefetch_redstone_feeds` runs
    // unconditionally, collecting two RedStone feeds and firing one bulk call.
    // After the fix: the prefetch is gated on `account.is_isolated`, so
    // bulk == 0 and single == 0.
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    // One adapter, three feeds.
    let redstone = setup_redstone_feeds(
        &t,
        &[("USDC", usd(1)), ("ETH", usd(2000)), ("WBTC", usd(60_000))],
    );
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");
    anchor_market_with_redstone(&t, &redstone, "WBTC");

    // BOB provides ETH and WBTC liquidity.
    t.supply(BOB, "ETH", 100.0);
    t.supply(BOB, "WBTC", 10.0);

    // ALICE has a plain (non-isolated) account.
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.borrow(ALICE, "WBTC", 0.1);

    // Snapshot counters before the repay.
    let rs = redstone_client(&t, &redstone);
    let bulk_before = rs.bulk_calls();
    let single_before = rs.single_calls();

    // Overpay both debts to force full closure (pool clamps to actual debt).
    t.repay_bulk(ALICE, &[("ETH", 2.0), ("WBTC", 0.5)]);

    let rs = redstone_client(&t, &redstone);
    assert_eq!(
        rs.bulk_calls() - bulk_before,
        0,
        "non-isolated full repay must fire zero bulk RedStone calls"
    );
    assert_eq!(
        rs.single_calls() - single_before,
        0,
        "non-isolated full repay must fire zero single RedStone calls"
    );
}
// ── Issue 2 regression ────────────────────────────────────────────────────────

#[test]
fn test_no_debt_withdraw_prefetch_covers_only_plan_assets() {
    // Account with NO debt and ≥2 RedStone-anchored supplies.  Withdraw part of
    // one asset.  Without debt the LTV and HF checks early-return, so only the
    // plan assets need pricing (the dust gate).
    //
    // Before the fix: the entrypoint `prefetch_redstone_feeds` collects ALL
    // supply keys (both RedStone feeds), fires one bulk call, then the dust gate
    // no-ops (already cached).  Net: bulk == 1.
    // After the fix: the prefetch covers only plan assets (one asset) — one feed
    // < MIN_BULK_FEEDS so no bulk is fired.  The dust gate's lazy path then does
    // one single read for that one feed.  Net: bulk == 0, single == 1.
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let redstone = setup_redstone_feeds(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");

    // ALICE supplies both assets; no debt.
    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 1.0);

    // Snapshot counters before the withdraw.
    let rs = redstone_client(&t, &redstone);
    let bulk_before = rs.bulk_calls();
    let single_before = rs.single_calls();

    // Withdraw part of USDC only — the plan has one feed (USDC).
    t.withdraw(ALICE, "USDC", 100.0);

    let rs = redstone_client(&t, &redstone);
    assert_eq!(
        rs.bulk_calls() - bulk_before,
        0,
        "no-debt single-asset withdraw must fire zero bulk calls (plan has 1 feed < MIN_BULK_FEEDS)"
    );
    assert_eq!(
        rs.single_calls() - single_before,
        1,
        "no-debt single-asset withdraw: dust gate fires exactly one single read for the plan asset"
    );
}
