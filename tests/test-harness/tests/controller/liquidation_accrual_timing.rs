//! A4-econ: does the liquidation path's "stale price + fresh index" asymmetry
//! give a liquidator anything?
//!
//! The premise under test is that indexes move mid-call while prices are pinned.
//! `pool::get_bulk_indexes` (contracts/pool/src/lib.rs:321) runs
//! `simulate_update_indexes(env, now, &sync)` — the same chunking and the same
//! `update_borrow_index` / `update_supply_index` / `calculate_supplier_rewards`
//! that `interest::global_sync` commits. So the plan already reads the
//! accrued-to-now index, and the accruals inside the repay and seize legs land
//! on `elapsed_ms == 0`.

use test_harness::{usd_cents, LendingTest, ALICE, BOB, LIQUIDATOR};

fn seed(t: &mut LendingTest) {
    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
}

/// A liquidator who pre-accrues with the permissionless `update_indexes` keeper
/// and then liquidates gets bit-identical delivery to one who liquidates
/// directly at the same ledger time. INV-IDX-04 time-consistency holds on this
/// path despite the accrual running three times inside one call.
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
