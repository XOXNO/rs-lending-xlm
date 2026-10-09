//! Specula CR-16 (oracle): `update_account_threshold(has_risks = true)` prices
//! every supplied token through the final 1.05 health-factor check, even for an
//! account with no debt. A stale feed on any supplied leg reverts the refresh
//! with `PriceFeedStale` for that account and for every account in the batch.
//! The `has_risks = false` path and the `health_factor` view never price a
//! debt-free account, which is the asymmetry the finding reports.
//!
//! Fixture: single-source Reflector `Twap(3)` feeds with a 900 s budget.
//! Backdating the ETH TWAP entry by 1000 s makes only ETH stale; USDC stays
//! fresh.

use test_harness::{assert_contract_error, errors, usd, LendingTest, ALICE, BOB};

/// Ledger time the scenarios start from. The harness builds near zero, too close
/// to backdate feeds by the offsets used below.
const T0: u64 = 100_000;
/// Older than the fixture's 900 s `max_stale_seconds`.
const STALE_OFFSET: i64 = -1_000;

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

/// The finding: a debt-free account cannot have its liquidation tuple refreshed
/// while any supplied leg's feed is stale, although the LTV-only refresh and the
/// health-factor view both skip pricing for the same account.
#[test]
fn cr16_debt_free_account_has_risks_reverts_on_stale_supply_feed() {
    let (t, alice, _bob) = debt_free_pair_with_stale_eth();

    // Control: the view path treats a debt-free account as unpriced (views.rs:33).
    assert_eq!(t.health_factor_raw(ALICE), i128::MAX);

    // Control: the LTV-only scope never loads prices.
    t.try_update_account_threshold(false, &[alice])
        .expect("has_risks = false must not price a debt-free account");

    // The finding: the final HF gate prices ETH and the stale feed aborts.
    assert_contract_error(
        t.try_update_account_threshold(true, &[alice]),
        errors::PRICE_FEED_STALE,
    );
}

/// Only supplied tokens are priced: Bob holds USDC alone, so the stale ETH feed
/// does not touch his refresh. Batched with Alice, the stale leg reverts both.
#[test]
fn cr16_stale_leg_reverts_the_whole_batch_but_not_other_accounts_alone() {
    let (t, alice, bob) = debt_free_pair_with_stale_eth();

    t.try_update_account_threshold(true, &[bob])
        .expect("an account whose supplied feeds are fresh refreshes");

    assert_contract_error(
        t.try_update_account_threshold(true, &[bob, alice]),
        errors::PRICE_FEED_STALE,
    );

    // Dropping the stale account from the batch is the operator workaround.
    t.try_update_account_threshold(true, &[bob])
        .expect("the batch without the stale account refreshes");
}

/// Once the feed is fresh again, the same call succeeds: the failure is a
/// transient availability cost, not a permanent liveness loss.
#[test]
fn cr16_refresh_recovers_once_the_feed_is_fresh() {
    let (mut t, alice, _bob) = debt_free_pair_with_stale_eth();

    assert_contract_error(
        t.try_update_account_threshold(true, &[alice]),
        errors::PRICE_FEED_STALE,
    );

    t.set_price("ETH", usd(2_000));
    t.try_update_account_threshold(true, &[alice])
        .expect("fresh feed: the full-tuple refresh succeeds");
}
