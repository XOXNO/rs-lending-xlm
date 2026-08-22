use test_harness::{
    assert_contract_error, errors, usd, usd_cents, usdc_preset, LendingTest, ALICE,
};
#[test]
fn test_validate_healthy_passes() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    t.assert_healthy(ALICE);
    let hf = t.health_factor(ALICE);
    assert!(hf > 1.0, "HF should be > 1.0, got {}", hf);
}
#[test]
fn test_validate_healthy_fails() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);

    t.set_price("USDC", usd_cents(50));

    t.assert_liquidatable(ALICE);
    let hf = t.health_factor(ALICE);
    assert!(hf < 1.0, "HF should be < 1.0 after price drop, got {}", hf);

    let result = t.try_withdraw(ALICE, "USDC", 1.0);
    assert_contract_error(result, errors::INSUFFICIENT_COLLATERAL);
}
#[test]
fn test_health_factor_no_debt_is_max() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 10_000.0);

    let hf_raw = t.health_factor_raw(ALICE);
    assert_eq!(hf_raw, i128::MAX, "HF with no debt should be i128::MAX");
}
#[test]
fn test_health_factor_changes_with_price() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 2.0);

    let hf_before = t.health_factor(ALICE);

    t.set_price("USDC", usd(2));

    let hf_after = t.health_factor(ALICE);
    assert!(
        hf_after > hf_before,
        "HF should increase when collateral price rises: before={}, after={}",
        hf_before,
        hf_after
    );
}
#[test]
fn test_pool_borrow_rate_increases_with_borrows() {
    let mut t = LendingTest::new().standard_two_asset().build();

    // Real ETH supply first: with only builder-seeded cash, utilization (and
    // therefore the rate) provably cannot move and the test would be green by
    // construction.
    t.supply(test_harness::BOB, "ETH", 100.0);
    let rate_before = t.pool_borrow_rate("ETH");

    t.supply(ALICE, "USDC", 500_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    let rate_after = t.pool_borrow_rate("ETH");
    assert!(
        rate_after > rate_before,
        "borrow rate must strictly rise with utilization: before={rate_before}, after={rate_after}"
    );
}
#[test]
fn test_borrow_exceeds_ltv_fails() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 10_000.0);

    let result = t.try_borrow(ALICE, "ETH", 4.0);
    assert_contract_error(result, errors::INSUFFICIENT_COLLATERAL);
}
#[test]
fn test_total_debt_zero_after_full_repay() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let debt_during = t.total_debt(ALICE);
    assert!(debt_during > 0.0, "should have debt after borrow");

    t.repay(ALICE, "ETH", 1.1);

    let debt_after = t.total_debt(ALICE);
    assert!(
        debt_after < 0.01,
        "debt should be ~0 after full repay, got {}",
        debt_after
    );
}
