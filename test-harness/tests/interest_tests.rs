extern crate std;

use test_harness::{days, eth_preset, usdc_preset, LendingTest, ALICE, BOB};

// ---------------------------------------------------------------------------
// 1. test_interest_accrues_on_borrow
// ---------------------------------------------------------------------------

#[test]
fn test_interest_accrues_on_borrow() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let debt_before = t.borrow_balance(ALICE, "ETH");

    t.advance_and_sync(days(365));

    let debt_after = t.borrow_balance(ALICE, "ETH");
    assert!(
        debt_after > debt_before,
        "debt should grow after 1 year: before={}, after={}",
        debt_before,
        debt_after
    );

    let interest = debt_after - debt_before;
    assert!(
        interest > 0.001,
        "interest should be non-trivial, got {}",
        interest
    );
}

// ---------------------------------------------------------------------------
// 2. test_interest_accrues_on_supply
// ---------------------------------------------------------------------------

#[test]
fn test_interest_accrues_on_supply() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Generating interest requires both supply and borrow.
    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(BOB, "ETH", 100.0);
    t.borrow(ALICE, "ETH", 10.0);

    let supply_before = t.supply_balance(BOB, "ETH");

    t.advance_and_sync(days(365));

    let supply_after = t.supply_balance(BOB, "ETH");
    assert!(
        supply_after > supply_before,
        "supply balance should grow from interest: before={}, after={}",
        supply_before,
        supply_after
    );
}

// ---------------------------------------------------------------------------
// 3. test_interest_rate_increases_with_utilization
// ---------------------------------------------------------------------------

#[test]
fn test_interest_rate_increases_with_utilization() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Low utilization.
    t.supply(ALICE, "ETH", 100.0);
    t.supply(BOB, "USDC", 500_000.0);
    t.borrow(BOB, "ETH", 1.0);

    let rate_low = t.pool_borrow_rate("ETH");

    // Higher utilization: borrow more, within the 75% LTV of $500k = $375k.
    t.borrow(BOB, "ETH", 80.0); // 81 ETH = $162k, within the LTV.

    let rate_high = t.pool_borrow_rate("ETH");
    assert!(
        rate_high > rate_low,
        "borrow rate should increase with utilization: low={}, high={}",
        rate_low,
        rate_high
    );
}

// ---------------------------------------------------------------------------
// 4. test_compound_interest_over_multiple_periods
// ---------------------------------------------------------------------------

#[test]
fn test_compound_interest_over_multiple_periods() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 5.0);

    let debt_start = t.borrow_balance(ALICE, "ETH");

    // Advance in four quarters.
    t.advance_and_sync(days(90));
    let debt_q1 = t.borrow_balance(ALICE, "ETH");

    t.advance_and_sync(days(90));
    let debt_q2 = t.borrow_balance(ALICE, "ETH");

    t.advance_and_sync(days(90));
    let debt_q3 = t.borrow_balance(ALICE, "ETH");

    t.advance_and_sync(days(90));
    let debt_q4 = t.borrow_balance(ALICE, "ETH");

    // Every quarter must accrue interest.
    assert!(debt_q1 > debt_start, "Q1: debt should grow");
    assert!(debt_q2 > debt_q1, "Q2: debt should grow");
    assert!(debt_q3 > debt_q2, "Q3: debt should grow");
    assert!(debt_q4 > debt_q3, "Q4: debt should grow");

    // Compounding (interest on interest) means later quarters accrue more.
    let interest_q1 = debt_q1 - debt_start;
    let interest_q4 = debt_q4 - debt_q3;
    assert!(
        interest_q4 >= interest_q1 * 0.99, // Allow small rounding.
        "later quarters should accrue at least as much interest (compound): q1={}, q4={}",
        interest_q1,
        interest_q4
    );
}

// ---------------------------------------------------------------------------
// 5. test_interest_zero_when_no_borrows
// ---------------------------------------------------------------------------

#[test]
fn test_interest_zero_when_no_borrows() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 100_000.0);

    let supply_before = t.supply_balance(ALICE, "USDC");

    t.advance_and_sync(days(365));

    let supply_after = t.supply_balance(ALICE, "USDC");

    // With no borrows, suppliers earn zero interest.
    let diff = (supply_after - supply_before).abs();
    assert!(
        diff < 0.01,
        "with no borrows, supply should not grow: before={}, after={}, diff={}",
        supply_before,
        supply_after,
        diff
    );
}

// ---------------------------------------------------------------------------
// 6. test_reserve_factor_splits_interest
// ---------------------------------------------------------------------------

#[test]
fn test_reserve_factor_splits_interest() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    let rev_before = t.snapshot_revenue("ETH");

    t.advance_and_sync(days(365));

    let rev_after = t.snapshot_revenue("ETH");
    assert!(
        rev_after > rev_before,
        "protocol should earn revenue from reserve factor: before={}, after={}",
        rev_before,
        rev_after
    );
}

// ---------------------------------------------------------------------------
// 7. test_advance_time_without_sync_stale
// ---------------------------------------------------------------------------

#[test]
fn test_advance_time_without_sync_stale() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // Advance time without syncing indexes.
    t.advance_time(days(30));

    // Views still work; values may be stale but must not panic.
    let hf = t.health_factor(ALICE);
    assert!(
        hf > 0.0,
        "health factor should be calculable even without sync"
    );

    let debt = t.borrow_balance(ALICE, "ETH");
    assert!(debt > 0.0, "borrow balance should be readable");
}

// ---------------------------------------------------------------------------
// 8. test_advance_and_sync_specific_markets
// ---------------------------------------------------------------------------

#[test]
fn test_advance_and_sync_specific_markets() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let debt_before = t.borrow_balance(ALICE, "ETH");

    // Sync only the ETH market.
    t.advance_and_sync_markets(days(365), &["ETH"]);

    let debt_after = t.borrow_balance(ALICE, "ETH");
    assert!(
        debt_after > debt_before,
        "syncing ETH market should accrue interest: before={}, after={}",
        debt_before,
        debt_after
    );
}
