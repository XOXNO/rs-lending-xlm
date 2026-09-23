//! Checks that the stale-price, fresh-index asymmetry on the liquidation path
//! gives a liquidator nothing.
//!
//! Indexes move during the call while prices stay fixed. `pool::get_bulk_indexes`
//! runs `simulate_update_indexes`, which uses the same chunking and the same
//! `accrue_step` that `interest::global_sync` commits. The plan projects the
//! accrued-to-now index without persisting it. The first mutation of each market
//! commits the same index; later mutations of that market have zero elapsed time.

use test_harness::{usd_cents, LendingTest, ALICE, BOB, LIQUIDATOR};

fn seed(t: &mut LendingTest) {
    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
}

/// A liquidator who pre-accrues with the permissionless `update_indexes` keeper
/// and then liquidates gets bit-identical delivery to one who liquidates
/// directly at the same ledger time. INV-IDX-04: the index projection and the
/// committed accrual share one calculation.
#[test]
fn preaccrual_does_not_change_liquidator_payoff() {
    // Run A: liquidate directly.
    let mut a = LendingTest::new().standard_two_asset_dust_disabled();
    seed(&mut a);
    a.advance_time(30 * 24 * 60 * 60);
    a.set_price("USDC", usd_cents(60));
    a.assert_liquidatable(ALICE);

    // The harness mints the liquidator's repayment inside `liquidate`, so create
    // the user first and compare the two runs' post-call balances: the minted
    // amount is identical in both, so any difference is delivery, not funding.
    a.get_or_create_user(LIQUIDATOR);
    let a_eth_before = a.token_balance_raw(LIQUIDATOR, "ETH");
    let a_usdc_before = a.token_balance_raw(LIQUIDATOR, "USDC");
    a.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    let a_spent = a.token_balance_raw(LIQUIDATOR, "ETH") - a_eth_before;
    let a_seized = a.token_balance_raw(LIQUIDATOR, "USDC") - a_usdc_before;

    // Run B: identical, but the liquidator commits the accrual first.
    let mut b = LendingTest::new().standard_two_asset_dust_disabled();
    seed(&mut b);
    b.advance_time(30 * 24 * 60 * 60);
    b.set_price("USDC", usd_cents(60));
    b.assert_liquidatable(ALICE);

    b.update_indexes_for(&["ETH", "USDC"]);

    b.get_or_create_user(LIQUIDATOR);
    let b_eth_before = b.token_balance_raw(LIQUIDATOR, "ETH");
    let b_usdc_before = b.token_balance_raw(LIQUIDATOR, "USDC");
    b.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    let b_spent = b.token_balance_raw(LIQUIDATOR, "ETH") - b_eth_before;
    let b_seized = b.token_balance_raw(LIQUIDATOR, "USDC") - b_usdc_before;

    // Without a liveness guard the two `assert_eq!`s below also hold when both
    // runs moved nothing at all.
    let minted = 10i128.pow(a.resolve_market("ETH").decimals);
    assert!(
        a_seized > 0 && a_spent < minted,
        "measurement must be live: net ETH delta={a_spent} of {minted} minted, seized={a_seized}"
    );

    assert_eq!(
        a_spent, b_spent,
        "debt-token delta differs: direct={} pre-accrued={}",
        a_spent, b_spent
    );
    assert_eq!(
        a_seized, b_seized,
        "collateral seized differs: direct={} pre-accrued={}",
        a_seized, b_seized
    );

    std::println!(
        "A4-econ preaccrual: direct spent={} seized={} | pre-accrued spent={} seized={}",
        a_spent,
        a_seized,
        b_spent,
        b_seized
    );
}
