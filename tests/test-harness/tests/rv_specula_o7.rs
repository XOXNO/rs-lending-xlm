//! Specula CR-16 (oracle): `update_account_threshold(has_risks = true)` priced
//! every supplied token through the final 1.05 health-factor check, even for an
//! account with no debt. A stale feed on any supplied leg reverted the refresh
//! with `PriceFeedStale` for that account and for every account in the batch,
//! while the `has_risks = false` path and the `health_factor` view never
//! priced a debt-free account. PR #196 (b172f2b, d6a88c8) skips the gate for a
//! debt-free account, so the refresh no longer prices it; the tests below are
//! flipped to that outcome.
//!
//! Fixture: single-source Reflector `Twap(3)` feeds with the harness budget.
//! Backdating the ETH TWAP entry by 2000 s makes only ETH stale; USDC stays
//! fresh.

use test_harness::{assert_contract_error, errors, usd, LendingTest, ALICE, BOB};

/// Ledger time the scenarios start from. The harness builds near zero, too close
/// to backdate feeds by the offsets used below.
const T0: u64 = 100_000;
/// Older than the harness `max_stale_seconds` (1_500 s since the oracle fix,
/// CR-10 `records + 2`).
const STALE_OFFSET: i64 = -2_000;

fn at(offset: i64) -> u64 {
    (T0 as i64 + offset) as u64
}

/// Moves the ledger clock without touching the sequence, so temporary mock
/// entries keep their TTL (the same approach as the staleness tests).
fn set_time(t: &LendingTest, ts: u64) {
    use soroban_sdk::testutils::Ledger as _;
    t.env.ledger().with_mut(|li| li.timestamp = ts);
}

/// Alice holds only ETH collateral and no debt; Bob holds only USDC and no debt.
/// ETH's TWAP feed is backdated past the stale budget, USDC's stays fresh.
fn debt_free_pair_with_stale_eth() -> (LendingTest, u64, u64) {
    let mut t = LendingTest::new().standard_two_asset_dust_disabled();
    set_time(&t, T0);
    t.refresh_oracle_prices();

    t.supply(ALICE, "ETH", 1.0);
    t.supply(BOB, "USDC", 1_000.0);
    let alice = t.resolve_account_id(ALICE);
    let bob = t.resolve_account_id(BOB);

    let eth = t.resolve_asset("ETH");
    t.mock_reflector_client()
        .set_twap_price_at(&eth, &usd(2_000), &at(STALE_OFFSET));
    (t, alice, bob)
}

/// The finding, fixed: a debt-free account's liquidation tuple refreshes while
/// a supplied leg's feed is stale, like the LTV-only refresh and the
/// health-factor view. A path that does price ETH still fails closed.
#[test]
fn cr16_debt_free_account_has_risks_refresh_skips_the_stale_supply_feed() {
    let (mut t, alice, _bob) = debt_free_pair_with_stale_eth();

    // The feed is stale: a borrow against ETH prices it and fails closed.
    assert_contract_error(t.try_borrow(ALICE, "USDC", 1.0), errors::PRICE_FEED_STALE);

    // Control: the view path treats a debt-free account as unpriced (views.rs:33).
    assert_eq!(t.health_factor_raw(ALICE), i128::MAX);

    // Control: the LTV-only scope never loads prices.
    t.try_update_account_threshold(false, &[alice])
        .expect("has_risks = false must not price a debt-free account");

    // Fixed by PR #196 (b172f2b): the full-tuple refresh skips the HF gate.
    t.try_update_account_threshold(true, &[alice])
        .expect("has_risks = true no longer prices a debt-free account");
}

/// Before PR #196 a debt-free account with a stale supplied leg reverted the
/// whole batch it was in. Now the batch refreshes, with or without it.
#[test]
fn cr16_stale_leg_no_longer_reverts_the_whole_batch() {
    let (t, alice, bob) = debt_free_pair_with_stale_eth();

    t.try_update_account_threshold(true, &[bob])
        .expect("an account whose supplied feeds are fresh refreshes");
    t.try_update_account_threshold(true, &[bob, alice])
        .expect("the batch with the stale debt-free account refreshes");
}

/// The refresh of a debt-free account succeeds whether the feed is stale or
/// fresh: it no longer waits on the feed (before PR #196 the stale case
/// reverted until the feed recovered).
#[test]
fn cr16_refresh_succeeds_with_a_stale_or_a_fresh_feed() {
    let (mut t, alice, _bob) = debt_free_pair_with_stale_eth();

    t.try_update_account_threshold(true, &[alice])
        .expect("stale feed: the full-tuple refresh succeeds");

    t.set_price("ETH", usd(2_000));
    t.try_update_account_threshold(true, &[alice])
        .expect("fresh feed: the full-tuple refresh succeeds");
}
