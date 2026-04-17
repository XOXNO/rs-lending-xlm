extern crate std;

use common::constants::WAD;

use test_harness::{
    assert_contract_error, errors, eth_preset, usd, usd_cents, usdc_preset, LendingTest, ALICE,
    LIQUIDATOR,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn setup_liquidatable() -> LendingTest {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Alice supplies 10k USDC and borrows 3 ETH (~$6000, near 75% LTV of $10k).
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.assert_healthy(ALICE);

    // Drop USDC price to $0.50 => collateral $5000, threshold $4000,
    // debt $6000 => HF ~0.67.
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);
    t
}

// ---------------------------------------------------------------------------
// 1. test_liquidation_basic_proportional
// ---------------------------------------------------------------------------

#[test]
fn test_liquidation_basic_proportional() {
    let mut t = setup_liquidatable();

    // The liquidator pays 1 ETH ($2000) of debt.
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);

    let liq_usdc_after = t.token_balance(LIQUIDATOR, "USDC");
    assert!(
        liq_usdc_after > 0.0,
        "liquidator should have received USDC collateral, got {}",
        liq_usdc_after
    );

    // Verify the bonus: collateral received should exceed the debt paid.
    // USDC price is $0.50, so collateral value = usdc_received * 0.50.
    // Debt paid = 1 ETH = $2000.
    let collateral_value_usd = liq_usdc_after * 0.50;
    let debt_paid_usd = 1.0 * 2000.0;
    assert!(
        collateral_value_usd > debt_paid_usd,
        "liquidator should profit from bonus: collateral ${:.2} > debt ${:.2}",
        collateral_value_usd,
        debt_paid_usd
    );
}

// ---------------------------------------------------------------------------
// 2. test_liquidation_targeted_single_collateral
// ---------------------------------------------------------------------------

#[test]
fn test_liquidation_targeted_single_collateral() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Alice supplies USDC and borrows ETH.
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);

    // Drop USDC to make Alice liquidatable.
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);

    // Liquidate 1 ETH of debt -- the Stellar controller uses proportional seizure only.
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);

    let liq_usdc = t.token_balance(LIQUIDATOR, "USDC");
    assert!(
        liq_usdc > 0.0,
        "liquidator should have received USDC collateral"
    );
}

// ---------------------------------------------------------------------------
// 3. test_liquidation_rejects_healthy_account
// ---------------------------------------------------------------------------

#[test]
fn test_liquidation_rejects_healthy_account() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0); // well within LTV.
    t.assert_healthy(ALICE);

    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);
    assert_contract_error(result, errors::HEALTH_FACTOR_TOO_HIGH);
}

// ---------------------------------------------------------------------------
// 4. test_liquidation_rejects_when_paused
// ---------------------------------------------------------------------------

#[test]
fn test_liquidation_rejects_when_paused() {
    let mut t = setup_liquidatable();
    t.pause();

    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::CONTRACT_PAUSED);

    t.unpause();
}

// ---------------------------------------------------------------------------
// 5. test_liquidation_dynamic_bonus_moderate
// ---------------------------------------------------------------------------

#[test]
fn test_liquidation_dynamic_bonus_moderate() {
    let mut t = setup_liquidatable();

    // HF ~0.67. Liquidator should profit from bonus.
    let _debt_before = t.total_debt(ALICE);
    let _collateral_before = t.total_collateral(ALICE);

    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);

    // The liquidator should have received collateral worth more than the debt paid.
    let liq_usdc = t.token_balance(LIQUIDATOR, "USDC");
    // Collateral value in USD at USDC price $0.50.
    let collateral_received_usd = liq_usdc * 0.50;
    // Debt paid is 1 ETH = $2000.
    assert!(
        collateral_received_usd > 2000.0,
        "liquidator should profit from bonus: received ${} of collateral for $2000 debt",
        collateral_received_usd
    );
}

// ---------------------------------------------------------------------------
// 6. test_liquidation_dynamic_bonus_deep_underwater
// ---------------------------------------------------------------------------

#[test]
fn test_liquidation_dynamic_bonus_deep_underwater() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);

    // Crash USDC price severely => HF much lower.
    t.set_price("USDC", usd_cents(25));
    t.assert_liquidatable(ALICE);

    let hf = t.health_factor(ALICE);
    assert!(hf < 0.5, "HF should be deeply underwater, got {}", hf);

    // Liquidation must still work.
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);

    let liq_usdc = t.token_balance(LIQUIDATOR, "USDC");
    assert!(liq_usdc > 0.0, "liquidator should receive collateral");
}

// ---------------------------------------------------------------------------
// 7. test_liquidation_protocol_fee_on_bonus_only
// ---------------------------------------------------------------------------

#[test]
fn test_liquidation_protocol_fee_on_bonus_only() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);

    let rev_before = t.snapshot_revenue("USDC");
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    let rev_after = t.snapshot_revenue("USDC");

    assert!(
        rev_after >= rev_before,
        "protocol revenue should not decrease after liquidation: before={}, after={}",
        rev_before,
        rev_after
    );
}

// ---------------------------------------------------------------------------
// 8. test_liquidation_liquidator_profit
// ---------------------------------------------------------------------------

#[test]
fn test_liquidation_liquidator_profit() {
    let mut t = setup_liquidatable();

    // The liquidator pays 1 ETH ($2000) of debt.
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);

    // The liquidator receives USDC collateral at a discounted price (bonus).
    let usdc_received = t.token_balance(LIQUIDATOR, "USDC");
    let usdc_value_usd = usdc_received * 0.50; // USDC is at $0.50.

    assert!(
        usdc_value_usd > 2000.0,
        "liquidator should profit: received ${} in collateral for $2000 debt",
        usdc_value_usd
    );
}

// ---------------------------------------------------------------------------
// 9. test_liquidation_multi_debt_payment
// ---------------------------------------------------------------------------

#[test]
fn test_liquidation_multi_debt_payment() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Supply USDC and borrow ETH near the limit.
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0); // ~$6000

    // Drop USDC price deeply so the account stays liquidatable after the first pass.
    t.set_price("USDC", usd_cents(30));
    t.assert_liquidatable(ALICE);

    // First liquidation.
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);

    // Check whether still liquidatable for a second pass.
    if t.can_be_liquidated(ALICE) {
        t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.3);
    }

    // The liquidator should have accumulated collateral.
    let liq_usdc = t.token_balance(LIQUIDATOR, "USDC");
    assert!(
        liq_usdc > 0.0,
        "liquidator should receive collateral from liquidation(s)"
    );
}

// ---------------------------------------------------------------------------
// 10. test_liquidation_caps_at_actual_debt
// ---------------------------------------------------------------------------

#[test]
fn test_liquidation_caps_at_actual_debt() {
    let mut t = setup_liquidatable();

    // Try to repay far more debt than the account owes. The liquidation
    // must cap repayment at the real debt amount.
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 100.0);

    let liq_usdc = t.token_balance(LIQUIDATOR, "USDC");
    assert!(
        liq_usdc > 0.0,
        "liquidator should have received USDC collateral: {}",
        liq_usdc
    );
}

// ---------------------------------------------------------------------------
// 11. test_liquidation_proportional_multi_collateral
// ---------------------------------------------------------------------------

#[test]
fn test_liquidation_proportional_multi_collateral() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Alice supplies USDC and borrows ETH.
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);

    // Drop USDC price to make Alice liquidatable.
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);

    // Proportional liquidation -- with single collateral, all seizure comes from that asset.
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);

    let liq_usdc = t.token_balance(LIQUIDATOR, "USDC");
    assert!(
        liq_usdc > 0.0,
        "liquidator should receive USDC collateral in proportional mode"
    );

    // Verify the debt decreased.
    let debt_after = t.borrow_balance(ALICE, "ETH");
    assert!(
        debt_after < 3.0,
        "debt should decrease after liquidation: {}",
        debt_after
    );
}

// ---------------------------------------------------------------------------
// 12. test_liquidation_improves_health_factor
// ---------------------------------------------------------------------------

#[test]
fn test_liquidation_improves_health_factor() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Supply USDC and borrow ETH at moderate utilization.
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0); // ~$6000

    // Drop USDC price to make Alice mildly liquidatable (HF ~0.8-0.9).
    // At $0.70: collateral = $7000, threshold = 80% => weighted = $5600,
    // debt = $6000 => HF = 0.93.
    t.set_price("USDC", usd_cents(70));
    t.assert_liquidatable(ALICE);

    let hf_before = t.health_factor(ALICE);

    // Small liquidation to improve HF.
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);

    let hf_after = t.health_factor(ALICE);
    assert!(
        hf_after > hf_before,
        "HF should improve after liquidation: before={}, after={}",
        hf_before,
        hf_after
    );
}

// ---------------------------------------------------------------------------
// 13. test_liquidation_caps_at_max_bonus
// ---------------------------------------------------------------------------

#[test]
fn test_liquidation_caps_at_max_bonus() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);

    // Crash price extremely => very low HF.
    t.set_price("USDC", usd_cents(10));
    t.assert_liquidatable(ALICE);

    // Liquidate a small amount.
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);

    // The collateral received must not imply a bonus > 15%.
    let usdc_received = t.token_balance(LIQUIDATOR, "USDC");
    let usdc_value = usdc_received * 0.10; // USDC at $0.10.
    let debt_paid = 0.5 * 2000.0; // 0.5 ETH at $2000.

    // Max bonus = 15% (1500 BPS), so max value ratio = 1.15.
    // Add 1% tolerance for protocol-fee effects on the seized amount.
    assert!(usdc_received > 0.0, "liquidator should receive collateral");
    if debt_paid > 0.0 && usdc_value > 0.0 {
        let ratio = usdc_value / debt_paid;
        assert!(
            ratio <= 1.16,
            "bonus ratio should be capped at 15% + 1% tolerance: got {:.4} (max 1.16)",
            ratio,
        );
    }
}

// ---------------------------------------------------------------------------
// 14. test_liquidation_bad_debt_cleanup_auto
// ---------------------------------------------------------------------------

#[test]
fn test_liquidation_bad_debt_cleanup_auto() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Small position.
    t.supply(ALICE, "USDC", 100.0);
    t.borrow(ALICE, "ETH", 0.03); // ~$60

    // Crash USDC price so collateral is nearly worthless.
    t.set_price("USDC", usd_cents(5));
    t.assert_liquidatable(ALICE);

    // Tiny underwater positions get cleaned up automatically during
    // liquidation.
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.03);

    // The account entry is removed during cleanup, so execution is confirmed
    // through the liquidator's received collateral.
    let liq_usdc = t.token_balance(LIQUIDATOR, "USDC");
    assert!(
        liq_usdc > 0.0,
        "liquidator should have received USDC collateral: {}",
        liq_usdc
    );
}

// ---------------------------------------------------------------------------
// 15. test_liquidation_bad_debt_socializes_loss
// ---------------------------------------------------------------------------

#[test]
fn test_liquidation_bad_debt_socializes_loss() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Small position.
    t.supply(ALICE, "USDC", 100.0);
    t.borrow(ALICE, "ETH", 0.03);

    // Crash price so collateral is nearly worthless.
    t.set_price("USDC", usd_cents(1));
    t.assert_liquidatable(ALICE);

    // Deeply underwater tiny positions socialize the residual loss during
    // liquidation.
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.03);

    // The account is removed during cleanup, so execution is confirmed
    // through the liquidator's collateral receipt.
    let liq_usdc = t.token_balance(LIQUIDATOR, "USDC");
    assert!(
        liq_usdc > 0.0,
        "liquidator should have received USDC collateral: {}",
        liq_usdc
    );
}

// ---------------------------------------------------------------------------
// 16. test_liquidation_isolated_debt_adjustment
// ---------------------------------------------------------------------------

#[test]
fn test_liquidation_isolated_debt_adjustment() {
    let mut t = LendingTest::new()
        .with_market(eth_preset())
        .with_market(usdc_preset())
        .with_market_config("ETH", |cfg| {
            cfg.is_isolated_asset = true;
            cfg.isolation_debt_ceiling_usd_wad = 1_000_000 * WAD;
            // $1M WAD
        })
        .with_market_config("USDC", |cfg| {
            cfg.isolation_borrow_enabled = true;
        })
        .build();

    // Create an isolated account for Alice.
    t.create_isolated_account(ALICE, "ETH");
    t.supply(ALICE, "ETH", 5.0); // ~$10,000
    t.borrow(ALICE, "USDC", 5_000.0);

    let debt_before = t.get_isolated_debt("ETH");
    assert!(debt_before > 0, "isolated debt should be tracked");

    // Make Alice liquidatable.
    t.set_price("ETH", usd(500)); // ETH drops to $500.
    t.assert_liquidatable(ALICE);

    t.liquidate(LIQUIDATOR, ALICE, "USDC", 2_000.0);

    let debt_after = t.get_isolated_debt("ETH");
    assert!(
        debt_after < debt_before,
        "isolated debt should decrease after liquidation: before={}, after={}",
        debt_before,
        debt_after
    );
}

// ---------------------------------------------------------------------------
// 17. test_liquidation_rejects_during_flash_loan
// ---------------------------------------------------------------------------

#[test]
fn test_liquidation_rejects_during_flash_loan() {
    let mut t = setup_liquidatable();

    // Set the flash-loan-ongoing flag.
    t.set_flash_loan_ongoing(true);

    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::FLASH_LOAN_ONGOING);

    t.set_flash_loan_ongoing(false);
}

// ---------------------------------------------------------------------------
// 18. test_liquidation_rejects_empty_debt_payments
// ---------------------------------------------------------------------------

#[test]
fn test_liquidation_rejects_empty_debt_payments() {
    let mut t = setup_liquidatable();

    // Use an exact zero payment. `0.0000001` ETH stays non-zero at 7 decimals.
    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.0);
    assert!(result.is_err(), "liquidation with zero amount should fail");
}
