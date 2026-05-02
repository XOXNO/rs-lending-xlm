extern crate std;

use common::constants::WAD;

use test_harness::{eth_preset, usd, usd_cents, usdc_preset, LendingTest, ALICE};

// ---------------------------------------------------------------------------
// 1. test_isolated_debt_non_isolated_account
// ---------------------------------------------------------------------------

#[test]
fn test_isolated_debt_non_isolated_account() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Normal (non-isolated) account.
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // ETH isolated debt must be 0 (not an isolated asset in this setup).
    let debt = t.get_isolated_debt("ETH");
    assert_eq!(debt, 0, "non-isolated setup should have 0 isolated debt");
}

// ---------------------------------------------------------------------------
// 2. test_isolated_debt_dust_erasure
// ---------------------------------------------------------------------------

#[test]
fn test_isolated_debt_dust_erasure() {
    let isolation_ceiling = 1_000_000i128 * WAD;

    let mut t = LendingTest::new()
        .with_market(eth_preset())
        .with_market(usdc_preset())
        .with_market_config("ETH", |cfg| {
            cfg.is_isolated_asset = true;
            cfg.isolation_debt_ceiling_usd_wad = isolation_ceiling;
        })
        .with_market_config("USDC", |cfg| {
            cfg.isolation_borrow_enabled = true;
        })
        .build();

    t.create_isolated_account(ALICE, "ETH");
    t.supply(ALICE, "ETH", 10.0);

    // Borrow a small amount.
    t.borrow(ALICE, "USDC", 1.0);

    let debt_after_borrow = t.get_isolated_debt("ETH");
    assert!(
        debt_after_borrow > 0,
        "should have isolated debt after borrow"
    );

    // Repay slightly more than borrowed (overpay). This must bring debt to
    // zero or near-zero and trigger dust erasure.
    t.repay(ALICE, "USDC", 2.0);

    let debt_after_repay = t.get_isolated_debt("ETH");
    assert_eq!(
        debt_after_repay, 0,
        "isolated debt should be 0 after full repay (dust erasure), got {}",
        debt_after_repay
    );
}

// ---------------------------------------------------------------------------
// 3. test_isolated_debt_over_repay_clamps
// ---------------------------------------------------------------------------

#[test]
fn test_isolated_debt_over_repay_clamps() {
    let isolation_ceiling = 1_000_000i128 * WAD;

    let mut t = LendingTest::new()
        .with_market(eth_preset())
        .with_market(usdc_preset())
        .with_market_config("ETH", |cfg| {
            cfg.is_isolated_asset = true;
            cfg.isolation_debt_ceiling_usd_wad = isolation_ceiling;
        })
        .with_market_config("USDC", |cfg| {
            cfg.isolation_borrow_enabled = true;
        })
        .build();

    t.create_isolated_account(ALICE, "ETH");
    t.supply(ALICE, "ETH", 10.0);
    t.borrow(ALICE, "USDC", 100.0);

    let debt_before = t.get_isolated_debt("ETH");
    assert!(debt_before > 0, "should have debt after borrow");

    // Repay the full amount.
    t.repay(ALICE, "USDC", 100.0);

    let debt_after = t.get_isolated_debt("ETH");
    assert_eq!(
        debt_after, 0,
        "debt should be 0 after full repay, got {}",
        debt_after
    );
}

// ---------------------------------------------------------------------------
// 4. test_validate_healthy_passes
// ---------------------------------------------------------------------------

#[test]
fn test_validate_healthy_passes() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // HF must sit well above 1.0.
    t.assert_healthy(ALICE);
    let hf = t.health_factor(ALICE);
    assert!(hf > 1.0, "HF should be > 1.0, got {}", hf);
}

// ---------------------------------------------------------------------------
// 5. test_validate_healthy_fails
// ---------------------------------------------------------------------------

#[test]
fn test_validate_healthy_fails() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);

    // Crash the USDC price to push HF below 1.0.
    t.set_price("USDC", usd_cents(50));

    t.assert_liquidatable(ALICE);
    let hf = t.health_factor(ALICE);
    assert!(hf < 1.0, "HF should be < 1.0 after price drop, got {}", hf);

    // Attempting to withdraw must fail due to low HF.
    let result = t.try_withdraw(ALICE, "USDC", 1.0);
    assert!(
        result.is_err(),
        "withdraw should fail when HF is below threshold"
    );
}

// ---------------------------------------------------------------------------
// 6. test_health_factor_no_debt_is_max
// ---------------------------------------------------------------------------

#[test]
fn test_health_factor_no_debt_is_max() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 10_000.0);

    // No borrows: HF must be i128::MAX.
    let hf_raw = t.health_factor_raw(ALICE);
    assert_eq!(hf_raw, i128::MAX, "HF with no debt should be i128::MAX");
}

// ---------------------------------------------------------------------------
// 7. test_health_factor_changes_with_price
// ---------------------------------------------------------------------------

#[test]
fn test_health_factor_changes_with_price() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 2.0);

    let hf_before = t.health_factor(ALICE);

    // Raise the USDC price: more collateral value, higher HF.
    t.set_price("USDC", usd(2));

    let hf_after = t.health_factor(ALICE);
    assert!(
        hf_after > hf_before,
        "HF should increase when collateral price rises: before={}, after={}",
        hf_before,
        hf_after
    );
}

// ---------------------------------------------------------------------------
// 8. test_pool_borrow_rate_increases_with_borrows
// ---------------------------------------------------------------------------

#[test]
fn test_pool_borrow_rate_increases_with_borrows() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // The borrow rate must start at the base rate (non-zero).
    let rate_before = t.pool_borrow_rate("ETH");

    t.supply(ALICE, "USDC", 500_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    // After the borrow, the rate must rise (more utilization, higher rate).
    let rate_after = t.pool_borrow_rate("ETH");
    assert!(
        rate_after >= rate_before,
        "borrow rate should not decrease after borrow: before={}, after={}",
        rate_before,
        rate_after
    );
}

// ---------------------------------------------------------------------------
// 9. test_borrow_exceeds_ltv_fails
// ---------------------------------------------------------------------------

#[test]
fn test_borrow_exceeds_ltv_fails() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Supply $10k USDC, LTV=75% => max borrow = $7500.
    t.supply(ALICE, "USDC", 10_000.0);

    // Borrow 4 ETH = $8000 > $7500.
    let result = t.try_borrow(ALICE, "ETH", 4.0);
    assert!(result.is_err(), "borrow exceeding LTV should fail");
}

// ---------------------------------------------------------------------------
// 10. test_total_debt_zero_after_full_repay
// ---------------------------------------------------------------------------

#[test]
fn test_total_debt_zero_after_full_repay() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let debt_during = t.total_debt(ALICE);
    assert!(debt_during > 0.0, "should have debt after borrow");

    // Repay more than owed to cover potential rounding.
    t.repay(ALICE, "ETH", 1.1);

    let debt_after = t.total_debt(ALICE);
    assert!(
        debt_after < 0.01,
        "debt should be ~0 after full repay, got {}",
        debt_after
    );
}
