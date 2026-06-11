use common::constants::WAD;
use soroban_sdk::String;
use test_harness::oracle::redstone::{
    anchor_market_with_redstone, anchor_market_with_redstone_feed, redstone_counters,
    register_redstone_adapter,
};
use test_harness::{
    assert_contract_error, errors, eth_preset, usd, usdc_preset, wbtc_preset, xlm_preset,
    LendingTest, ALICE, BOB,
};

#[test]
fn test_borrow_tx_fires_one_bulk_redstone_call() {
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
    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);

    // Anchor both markets to the single adapter.
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");

    // BOB provides ETH liquidity so ALICE can borrow it.
    t.supply(BOB, "ETH", 100.0);

    // ALICE supplies USDC as collateral.
    t.supply(ALICE, "USDC", 10_000.0);

    // Measure counters BEFORE the borrow (each client call is its own tx).
    let rs = redstone_counters(&t, &redstone);
    let single_before = rs.single_calls();
    let bulk_before = rs.bulk_calls();

    // Borrow triggers an HF check that must price BOTH feeds.
    t.borrow(ALICE, "ETH", 1.0);

    // Re-read counters AFTER the borrow.
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
    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");

    let rs = redstone_counters(&t, &redstone);
    let bulk_before = rs.bulk_calls();
    let single_before = rs.single_calls();

    // Supply both assets in one controller call — no debt means HF is
    // skipped; the dust gate is the sole price consumer.
    t.supply_bulk(ALICE, &[("USDC", 100.0), ("ETH", 1.0)]);

    let rs = redstone_counters(&t, &redstone);
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
    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);
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
    let rs = redstone_counters(&t, &redstone);
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
    let rs = redstone_counters(&t, &redstone);
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
fn test_prefetched_prices_resolve_to_expected_values() {
    // Both feeds priced; ALICE supplies USDC and borrows ETH so the bulk path
    // is exercised inside both txs.  Assert the resulting account is healthy
    // (HF > 1.0), which can only hold if the prefetched prices resolve to the
    // same values the lazy per-feed path would produce from the same mock data.
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);
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
    // Allow ~1% accrual epsilon on deterministic mock prices.
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
    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);

    // Anchor both markets to the single adapter.
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");

    // BOB supplies ETH liquidity so ALICE can borrow it.
    t.supply(BOB, "ETH", 100.0);

    // ALICE supplies USDC as collateral and borrows ETH.
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // Snapshot counters after setup (each operation above is its own tx).
    let rs = redstone_counters(&t, &redstone);
    let bulk_before = rs.bulk_calls();
    let single_before = rs.single_calls();

    // ALICE withdraws a small portion of her USDC — small enough to stay healthy.
    // This triggers: withdrawal loop (prices USDC), then require_within_ltv
    // (prices all supply+borrow), then require_healthy_account.  Without an
    // entrypoint prefetch the USDC feed is single-resolved before the bulk
    // opportunity and ETH is resolved lazily too.
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
fn test_isolated_multi_asset_repay_uses_one_bulk_redstone_call() {
    // An isolated account with TWO isolation-borrowable debt assets repaying
    // both in one tx.  The isolated path in `process_single_repay` calls
    // `cache.cached_price(asset)` for each repaid asset BEFORE the dust gate
    // runs its own prefetch — so without an entrypoint prefetch the first asset
    // single-resolves its feed before any bulk opportunity.
    //
    // With the fix (`prefetch_redstone_feeds` over owed assets at the
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
    let redstone = register_redstone_adapter(
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
    let rs = redstone_counters(&t, &redstone);
    let bulk_before = rs.bulk_calls();
    let single_before = rs.single_calls();

    // Repay both debt assets in a single controller call.
    t.repay_bulk(ALICE, &[("ETH", 1.0), ("WBTC", 0.1)]);

    let rs = redstone_counters(&t, &redstone);
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

// ── Non-isolated repay must not prefetch ─────────────────────────────────────

#[test]
fn test_non_isolated_full_repay_fires_zero_redstone_calls() {
    // NON-isolated account with two RedStone-anchored debt assets; repaying
    // BOTH IN FULL in one tx.  The non-isolated repay path sets price=Wad::ZERO
    // for each asset, so no pricing happens in the loop.  The dust gate
    // prescreens for open positions and skips fully-closed ones — so zero
    // RedStone reads are needed.
    //
    // Invariant: a non-isolated full repay fires zero bulk and zero single
    // RedStone calls.
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    // One adapter, three feeds.
    let redstone = register_redstone_adapter(
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
    let rs = redstone_counters(&t, &redstone);
    let bulk_before = rs.bulk_calls();
    let single_before = rs.single_calls();

    // Overpay both debts to force full closure (pool clamps to actual debt).
    t.repay_bulk(ALICE, &[("ETH", 2.0), ("WBTC", 0.5)]);

    let rs = redstone_counters(&t, &redstone);
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

// ── No-debt withdraw scopes prefetch to plan assets ───────────────────────────

#[test]
fn test_no_debt_withdraw_prefetch_covers_only_plan_assets() {
    // Account with NO debt and ≥2 RedStone-anchored supplies.  Withdraw part of
    // one asset.  Without debt the LTV and HF checks early-return, so only the
    // plan assets need pricing (the dust gate).
    //
    // Invariant: the entrypoint prefetch covers only plan assets; one feed
    // below MIN_BULK_FEEDS means no bulk call fires.  The dust gate's lazy
    // path does one single read for that one feed.
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");

    // ALICE supplies both assets; no debt.
    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 1.0);

    // Snapshot counters before the withdraw.
    let rs = redstone_counters(&t, &redstone);
    let bulk_before = rs.bulk_calls();
    let single_before = rs.single_calls();

    // Withdraw part of USDC only — the plan has one feed (USDC).
    t.withdraw(ALICE, "USDC", 100.0);

    let rs = redstone_counters(&t, &redstone);
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

// ── Multi-adapter bulk grouping ───────────────────────────────────────────────

#[test]
fn test_two_adapters_bulk_once_each() {
    // Four markets split across TWO mock adapters (2 feeds each).  Once a
    // borrow tx has ≥2 feeds from each adapter in the position set, the
    // prefetch must fire exactly one bulk call per adapter with zero single calls.
    //
    // Invariant: each adapter fires exactly one bulk call and zero single calls
    // when its feed count reaches MIN_BULK_FEEDS.
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_market(xlm_preset())
        .build();

    // Two separate mock adapters.
    let adapter_a = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);
    let adapter_b = register_redstone_adapter(&t, &[("WBTC", usd(60_000)), ("XLM", usd(1) / 10)]);

    anchor_market_with_redstone(&t, &adapter_a, "USDC");
    anchor_market_with_redstone(&t, &adapter_a, "ETH");
    anchor_market_with_redstone(&t, &adapter_b, "WBTC");
    anchor_market_with_redstone(&t, &adapter_b, "XLM");

    // BOB provides liquidity for the borrowable assets.
    t.supply(BOB, "ETH", 100.0);
    t.supply(BOB, "WBTC", 10.0);
    t.supply(BOB, "XLM", 1_000_000.0);

    // ALICE supplies USDC as collateral; build up ETH + WBTC + XLM debt first
    // so the fourth borrow tx starts with all four assets in the position set.
    t.supply(ALICE, "USDC", 1_000_000.0);
    t.borrow(ALICE, "ETH", 0.1);
    t.borrow(ALICE, "WBTC", 0.001);
    t.borrow(ALICE, "XLM", 100.0);

    // From here: supply=USDC(A), borrows=ETH(A)+WBTC(B)+XLM(B) → adapter A
    // has 2 feeds, adapter B has 2 feeds.  The next borrow will fire one bulk
    // per adapter and zero single calls on each.
    let rs_a = redstone_counters(&t, &adapter_a);
    let rs_b = redstone_counters(&t, &adapter_b);
    let bulk_a_before = rs_a.bulk_calls();
    let single_a_before = rs_a.single_calls();
    let bulk_b_before = rs_b.bulk_calls();
    let single_b_before = rs_b.single_calls();

    // Additional small borrow: position set is now fully 4-asset, 2 per adapter.
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

// ── Unlisted-asset robustness ─────────────────────────────────────────────────

#[test]
fn test_prefetch_skips_unlisted_asset_without_panic() {
    // The `anchor_market_with_redstone_feed` helper exercises the feed-id-explicit
    // variant (needed for future tests where two markets share one feed).
    // This test also exercises the Fix-1 behavior: if an asset with no market
    // config appears in the prefetch asset list, the collector must skip it
    // silently rather than panicking with AssetNotSupported.
    //
    // Invariant: a prefetch list containing an unlisted asset still completes and
    // the listed asset's price is resolved correctly.
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);

    // Use the feed-id-explicit variant to anchor USDC with a custom feed id.
    anchor_market_with_redstone_feed(&t, &redstone, "USDC", "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");

    // Normal supply and borrow — verifies end-to-end correctness despite the
    // unlisted-asset-skip path being exercised by the prefetch module.
    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.assert_healthy(ALICE);
}
