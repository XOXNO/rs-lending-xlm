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
    // Two RedStone-anchored markets on the SAME adapter; a borrow's HF check
    // prices both feeds and must dispatch exactly one bulk call.
    //
    // The borrow tx contains three internal prefetch call sites:
    //   1. contracts/controller/src/positions/borrow.rs entrypoint: explicit
    //      prefetch with [supply_assets + borrow_assets] before the HF check.
    //   2. helpers/math.rs risk-totals body (calculate_account_risk_totals_body):
    //      a second prefetch_redstone_feeds call for the same feed set.
    //   3. helpers/math.rs min-borrow-collateral body: a third prefetch site.
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
fn test_multi_asset_supply_fires_zero_redstone_calls() {
    // Pure supply no longer runs per-asset dust pricing; even a multi-asset
    // deposit must not touch RedStone when the account carries no debt.
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
    // the same error the per-feed-only path produces.
    //
    // Counter deltas are asserted on committed txs only: try_borrow rolls
    // back its own storage changes, so the failed tx leaves no bumps.
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

    // Setup supplies no longer price collateral, so counters stay flat.
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

// ── Full repay must not prefetch ─────────────────────────────────────────────

#[test]
fn test_full_repay_fires_zero_redstone_calls() {
    // Account with two RedStone-anchored debt assets; repaying BOTH IN FULL in
    // one tx. The repay path sets price=Wad::ZERO for each asset, so no
    // pricing happens in the loop. The dust gate prescreens for open positions
    // and skips fully-closed ones — so zero RedStone reads are needed.
    //
    // Invariant: a full repay fires zero bulk and zero single RedStone calls.
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

    // ALICE has a standard account.
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
        "full repay must fire zero bulk RedStone calls"
    );
    assert_eq!(
        rs.single_calls() - single_before,
        0,
        "full repay must fire zero single RedStone calls"
    );
}

// ── No-debt withdraw skips oracle prefetch ────────────────────────────────────

#[test]
fn test_no_debt_withdraw_fires_zero_redstone_calls() {
    // Debt-free withdrawals skip LTV/HF/min-collateral gates and the withdraw
    // prefetch, so no oracle reads run even when the account holds multiple
    // RedStone-anchored supplies.
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
    // Multi-asset full close in one tx: enough feeds to cross MIN_BULK_FEEDS (2)
    // on a single adapter, but debt-free exits must still avoid oracle reads.
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
    // An asset with no market config in the prefetch list is skipped
    // silently rather than panicking with AssetNotSupported: the prefetch
    // completes and the listed asset's price still resolves correctly.
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

// ── Shared-feed invariant ─────────────────────────────────────────────

#[test]
fn test_shared_feed_two_assets_single_redstone_call() {
    // Two markets whose primary oracle is the same (adapter, feed_id),
    // configured via RedStone Single strategy.  The collector dedupes to a
    // 1-feed group below MIN_BULK_FEEDS, so no bulk fires; the first lazy
    // read warms the prefetch map and the second consumer is a cache hit:
    // total RedStone calls == 1.
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // One adapter, one price feed "SHARED" priced at $1.
    let redstone = register_redstone_adapter(&t, &[("SHARED", usd(1))]);
    let feed_id = String::from_str(&t.env, "SHARED");

    // Configure both USDC and ETH with RedStone Single strategy on "SHARED".
    // Both markets now resolve to the same (adapter, feed_id) — the degenerate
    // shared-feed case that exposes the 2-call bug.
    let usdc_cfg = redstone_single_config(
        &redstone,
        &feed_id,
        DEFAULT_TOLERANCE.first_upper_bps,
        DEFAULT_TOLERANCE.last_upper_bps,
    );
    let eth_cfg = redstone_single_config(
        &redstone,
        &feed_id,
        DEFAULT_TOLERANCE.first_upper_bps,
        DEFAULT_TOLERANCE.last_upper_bps,
    );
    t.configure_market_oracle(&t.resolve_asset("USDC"), &usdc_cfg);
    t.configure_market_oracle(&t.resolve_asset("ETH"), &eth_cfg);

    // BOB supplies ETH so ALICE can borrow.
    // With SHARED=$1 and ETH having 8 decimals, borrow at least $10 to clear
    // the dust floor (MIN_DUST_FLOOR_WAD = $10).
    t.supply(BOB, "ETH", 10_000.0);
    t.supply(ALICE, "USDC", 1_000_000.0);

    let rs = redstone_counters(&t, &redstone);
    let single_before = rs.single_calls();
    let bulk_before = rs.bulk_calls();

    // The borrow's HF check prices both USDC and ETH, which share one
    // (adapter, feed_id): exactly 1 single call total.
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

// ── Per-flow invariant pins ───────────────────────────────────────────────────

#[test]
fn test_liquidation_fires_one_bulk_redstone_call() {
    // Two RedStone-anchored markets on one adapter; ALICE is made liquidatable
    // by raising ETH price so her debt value exceeds her collateral weight.
    // Liquidation has no entrypoint prefetch: the risk-totals pass inside
    // calculate_account_risk_totals_body is its only bulk site, and this test
    // pins it.
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");

    // ALICE supplies USDC and borrows ETH near max LTV.
    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.assert_healthy(ALICE);

    // Move ETH price UP on both sources so HF < 1.  Both prices must stay
    // within DEFAULT_TOLERANCE of each other (5% max).  Raising to $4000 makes
    // the ETH debt worth $12 000 against $10 000 collateral.
    let rs_client = redstone_counters(&t, &redstone);
    rs_client.set_price(&String::from_str(&t.env, "ETH"), &usd(4000));
    t.set_price("ETH", usd(4000));

    t.assert_liquidatable(ALICE);

    // Snapshot before the liquidation tx.
    let rs = redstone_counters(&t, &redstone);
    let bulk_before = rs.bulk_calls();
    let single_before = rs.single_calls();

    // Liquidate part of ALICE's ETH debt.
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
    // Two markets with RedStone as the sole/primary source
    // (OracleStrategy::Single) — the production BTC/ETH shape. Pins the
    // collector's primary-source branch.
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // One adapter, two feeds.
    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);
    let usdc_feed = String::from_str(&t.env, "USDC");
    let eth_feed = String::from_str(&t.env, "ETH");

    // Both markets use RedStone Single strategy (primary = RedStone, no anchor).
    let usdc_cfg = redstone_single_config(
        &redstone,
        &usdc_feed,
        DEFAULT_TOLERANCE.first_upper_bps,
        DEFAULT_TOLERANCE.last_upper_bps,
    );
    let eth_cfg = redstone_single_config(
        &redstone,
        &eth_feed,
        DEFAULT_TOLERANCE.first_upper_bps,
        DEFAULT_TOLERANCE.last_upper_bps,
    );
    t.configure_market_oracle(&t.resolve_asset("USDC"), &usdc_cfg);
    t.configure_market_oracle(&t.resolve_asset("ETH"), &eth_cfg);

    // BOB provides ETH liquidity.
    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10_000.0);

    // Snapshot counters before the borrow.
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

    // Value assertions: Single strategy uses feed price directly.
    // USDC collateral ≈ $10 000, ETH debt ≈ $2 000 — allow 1% accrual epsilon.
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
    // One RedStone-anchored market; ALICE supplies asset X and borrows the
    // same X, so both position sides share one (adapter, feed_id). The
    // collector dedupes to a 1-feed group below MIN_BULK_FEEDS — no bulk —
    // and the first lazy read warms the prefetch map, so the second side is
    // a cache hit: total RedStone calls == 1.
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1))]);
    anchor_market_with_redstone(&t, &redstone, "USDC");

    // BOB supplies USDC liquidity so ALICE can borrow.
    t.supply(BOB, "USDC", 100_000.0);
    t.supply(ALICE, "USDC", 10_000.0);

    let rs = redstone_counters(&t, &redstone);
    let single_before = rs.single_calls();
    let bulk_before = rs.bulk_calls();

    // Borrow USDC against USDC: one distinct feed, one consumer.
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
    // Adapter A: 2 anchored feeds.  Adapter B: 1 anchored feed.
    // A flow pricing all three asserts: A fires bulk+0single; B fires 0bulk+1single.
    // Invariant: each adapter fires ≤1 call total, regardless of feed count.
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

    // BOB provides ETH and WBTC liquidity.
    t.supply(BOB, "ETH", 100.0);
    t.supply(BOB, "WBTC", 10.0);

    // ALICE supplies USDC and borrows ETH + WBTC — prices all three assets.
    t.supply(ALICE, "USDC", 1_000_000.0);
    t.borrow(ALICE, "ETH", 0.1);

    // Snapshot before the second borrow which will price USDC+ETH+WBTC.
    let rs_a = redstone_counters(&t, &adapter_a);
    let rs_b = redstone_counters(&t, &adapter_b);
    let bulk_a_before = rs_a.bulk_calls();
    let single_a_before = rs_a.single_calls();
    let bulk_b_before = rs_b.bulk_calls();
    let single_b_before = rs_b.single_calls();

    t.borrow(ALICE, "WBTC", 0.001);

    let rs_a = redstone_counters(&t, &adapter_a);
    let rs_b = redstone_counters(&t, &adapter_b);

    // Adapter A: 2 feeds → bulk fires once, zero single.
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

    // Adapter B: 1 feed < MIN_BULK_FEEDS → no bulk, one single (lazy-warmed).
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
fn test_committed_bulk_failure_degrades_to_singles() {
    // Two anchored markets on one adapter; ETH feed is removed after
    // configure-time validation so it is absent at runtime.  A view that
    // prices both supplies exercises the same bulk-fail → lazy-single path
    // supply used to hit via the removed dust gate.
    //
    // Inside the view the sequence is:
    //   1. prefetch bulk [USDC, ETH] → ETH absent → whole-call Err → bulk
    //      counter bump rolls back with the Err frame → prefetch map empty.
    //   2. USDC cached_price: lazy single → feed found → USDC single commits.
    //   3. ETH cached_price: lazy single → ETH absent → Err frame rolls back;
    //      View policy falls back to the Reflector primary.
    //
    // Observable after the committed view:
    //   • view succeeds (View tolerates missing anchor).
    //   • committed bulk delta == 0 (Err frame rolled back).
    //   • committed single delta == 1 (only USDC's read committed).
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

    let rs = redstone_counters(&t, &redstone);
    let bulk_before = rs.bulk_calls();
    let single_before = rs.single_calls();

    let _ = t.total_collateral(ALICE);

    let rs = redstone_counters(&t, &redstone);
    assert_eq!(
        rs.bulk_calls() - bulk_before,
        0,
        "bulk attempt Err-frame rolls back with its counter bump"
    );
    assert_eq!(
        rs.single_calls() - single_before,
        1,
        "only USDC's successful single read commits; ETH's Err-frame rolls back"
    );
}

#[test]
fn test_stale_payload_through_bulk_is_still_rejected() {
    // Two anchored markets on one adapter.  ONE feed (ETH) has stale timestamps
    // (older than DEFAULT_REDSTONE_MAX_STALE_SECONDS=900s).  The bulk prefetch
    // SUCCEEDS (the mock returns data for both feeds regardless of age) and
    // caches the stale payload.  Policy enforcement runs at compose time:
    //
    //   (a) Supply (RiskDecreasing): stale anchor → anchor_is_usable=false →
    //       fallback_to_primary → supply succeeds.  Bulk delta == 1.
    //
    //   (b) Borrow (RiskIncreasing): stale anchor → anchor_is_usable panics
    //       PriceFeedStale (#207).  The failed tx rolls back its counter bumps.
    //
    // This pins that the bulk path does NOT bypass the staleness policy — only
    // the raw payload is cached; policy reruns on every consume.
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");

    // Set ETH's timestamps to 950 seconds in the past — stale for the 900s window.
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

    let rs = redstone_counters(&t, &redstone);
    let bulk_before = rs.bulk_calls();
    let single_before = rs.single_calls();

    // total_collateral_in_usd prefetches [USDC, ETH]:
    //   bulk succeeds → cache holds stale ETH payload.
    // View policy tolerates stale anchors, so the read completes.
    let _ = t.total_collateral(ALICE);

    let rs = redstone_counters(&t, &redstone);
    assert_eq!(
        rs.bulk_calls() - bulk_before,
        1,
        "bulk prefetch fires once for [USDC, ETH] even when ETH anchor is stale"
    );
    assert_eq!(
        rs.single_calls() - single_before,
        0,
        "no single calls: bulk cached both payloads"
    );

    // (b) Borrow: RiskIncreasing + stale ETH anchor → PriceFeedStale panic.
    // The failed tx rolls back its counter bumps — no counter assertions here.
    let result = t.try_borrow(ALICE, "ETH", 0.001);
    assert_contract_error(result, errors::OracleError::PriceFeedStale as u32);
}

#[test]
fn test_disabled_market_panics_same_through_prefetch() {
    // One of two RedStone-anchored markets gets disabled.  A borrow that
    // touches the disabled market must panic PairNotActive (#12) — the same
    // error as before the bulk-prefetch feature.  This pins that prefetching
    // a disabled market's feed does NOT bypass the status check in token_price.
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

    // Disable the ETH market.
    t.env.as_contract(&t.controller, || {
        let key = controller::types::ControllerKey::Market(t.resolve_asset("ETH"));
        let mut market: controller::types::MarketConfig =
            t.env.storage().persistent().get(&key).unwrap();
        market.status = controller::types::MarketStatus::Disabled;
        t.env.storage().persistent().set(&key, &market);
    });

    // Attempt a withdrawal-with-debt: HF check prices both assets including
    // disabled ETH → token_price panics PairNotActive.
    let result = t.try_borrow(ALICE, "ETH", 0.001);
    assert_contract_error(result, errors::GenericError::PairNotActive as u32);
}

// ── Strategy bulk-prefetch invariant ─────────────────────────────────────────

#[test]
fn test_multiply_fires_one_bulk_redstone_call() {
    // USDC (collateral) and ETH (debt) anchored on the same adapter.
    // A multiply tx borrows ETH, swaps to USDC, and deposits — the strategy
    // prices both tokens and runs the LTV/HF check. The entrypoint prefetch
    // (positions + collateral + debt) runs before the first price read, so
    // both feeds resolve from the bulk cache.
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETH", usd(2000))]);
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "ETH");

    // Fund the mock router with USDC so the ETH→USDC swap succeeds.
    // 1 ETH (7 decimals) after 9bps flash fee.
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

    // Multiply: borrows 1 ETH, swaps to USDC collateral.
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
    // Three RedStone-anchored markets on one adapter.  ALICE has supply positions
    // for all three.  total_collateral_in_usd loops over 3 markets and must fire
    // exactly one bulk call rather than 3 single reads.
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

    // ALICE supplies all three so the view iterates a 3-asset map.
    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 1.0);
    t.supply(ALICE, "WBTC", 0.1);

    let rs = redstone_counters(&t, &redstone);
    let bulk_before = rs.bulk_calls();
    let single_before = rs.single_calls();

    // Call total_collateral_in_usd: should bulk-fetch the 3 feeds once.
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

    // Same invariant for total_borrow_in_usd: ALICE borrows two RedStone-anchored
    // assets so the view iterates a 2-entry debt map and fires one bulk call.
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
