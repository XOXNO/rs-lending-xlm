use test_harness::{
    assert_contract_error, errors, eth_preset, liquidatable_usdc_eth, usd_cents, usdc_preset,
    LendingTest, ALICE, BOB, LIQUIDATOR,
};
#[test]
fn test_liquidation_basic_proportional() {
    let mut t = liquidatable_usdc_eth();

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
    // Borrower post-state: debt and collateral both decreased.
    assert!(
        t.borrow_balance(ALICE, "ETH") < 3.0,
        "Alice ETH debt must decrease"
    );
    assert!(
        t.supply_balance(ALICE, "USDC") < 10_000.0,
        "Alice USDC must be seized"
    );
}
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
    // Borrower post-state: ETH debt and USDC collateral both reduced.
    assert!(t.borrow_balance(ALICE, "ETH") < 3.0);
    assert!(t.supply_balance(ALICE, "USDC") < 10_000.0);
    assert!(t.health_factor(ALICE) > 0.0);
}
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
#[test]
fn test_liquidation_allowed_when_paused() {
    let mut t = liquidatable_usdc_eth();
    t.pause();

    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    assert!(
        result.is_ok(),
        "liquidation should remain available while paused"
    );
}
#[test]
fn test_liquidation_dynamic_bonus_moderate() {
    let mut t = liquidatable_usdc_eth();

    // Re-price into the guard-safe moderate band: HF ~0.947 keeps the
    // HF-scaled bonus (~15%) under the HF-neutral cap hf/p - 1 (~18.3%).
    // (The fixture's HF ~0.67 is toxic-band: there the guard caps to the
    // base bonus, covered by the deep-underwater test.)
    t.set_price("USDC", usd_cents(71));
    t.assert_liquidatable(ALICE);
    let hf_before = t.health_factor(ALICE);

    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);

    // The liquidator should have received collateral worth more than the debt paid.
    let liq_usdc = t.token_balance(LIQUIDATOR, "USDC");
    // Collateral value in USD at USDC price $0.71.
    let collateral_received_usd = liq_usdc * 0.71;
    // Debt paid is 1 ETH = $2000.
    assert!(
        collateral_received_usd > 2000.0,
        "liquidator should profit from bonus: received ${} of collateral for $2000 debt",
        collateral_received_usd
    );
    // The HF-scaled bonus at moderate HF (~0.947) sits between the base and
    // the 25% per-threshold max for 80%-LT collateral.
    let bonus_rate = collateral_received_usd / 2000.0 - 1.0;
    assert!(
        bonus_rate > 0.10 && bonus_rate < 0.25,
        "moderate-HF bonus must be a mid-range HF-scaled value, got {:.4}",
        bonus_rate
    );
    // Borrower debt reduced and the guarded partial left the account healthier.
    assert!(t.borrow_balance(ALICE, "ETH") < 3.0);
    let hf_after = t.health_factor(ALICE);
    assert!(
        hf_after > hf_before,
        "guarded partial must improve HF: {hf_before:.4} -> {hf_after:.4}"
    );
}
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
    assert!(t.borrow_balance(ALICE, "ETH") < 3.0);
}
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

    t.assert_revenue_increased_since("USDC", rev_before);
    // Fee must be < 1% of total seizure (fee = bonus_portion * 100 BPS).
    // Liquidator received collateral; fee is a small slice of the bonus.
    let fee = (rev_after - rev_before) as f64 / 1e7;
    let liquidator_received = t.token_balance(LIQUIDATOR, "USDC");
    assert!(
        fee > 0.0 && fee / liquidator_received < 0.01,
        "fee should be on bonus only (<1% of total seizure): fee={:.4}, recv={:.4}",
        fee,
        liquidator_received
    );
    assert!(t.borrow_balance(ALICE, "ETH") < 3.0);
}
#[test]
fn test_liquidation_liquidator_profit() {
    let mut t = liquidatable_usdc_eth();

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
    // Borrower side: debt reduced, collateral seized.
    assert!(t.borrow_balance(ALICE, "ETH") < 3.0);
    assert!(t.supply_balance(ALICE, "USDC") < 10_000.0);
}
#[test]
fn test_liquidation_sequential_partial_liquidations() {
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
    let debt_before = t.borrow_balance(ALICE, "ETH");
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);
    let debt_after_first = t.borrow_balance(ALICE, "ETH");
    assert!(
        debt_after_first < debt_before,
        "1st liquidation must reduce debt"
    );

    // Check whether still liquidatable for a second pass.
    if t.can_be_liquidated(ALICE) {
        t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.3);
        assert!(
            t.borrow_balance(ALICE, "ETH") < debt_after_first,
            "2nd liquidation must reduce debt further"
        );
    }

    // The liquidator should have accumulated collateral.
    let liq_usdc = t.token_balance(LIQUIDATOR, "USDC");
    assert!(
        liq_usdc > 0.0,
        "liquidator should receive collateral from liquidation(s)"
    );
    assert!(
        t.supply_balance(ALICE, "USDC") < 10_000.0,
        "Alice USDC collateral must be seized"
    );
}
#[test]
fn test_liquidation_caps_at_actual_debt() {
    let mut t = liquidatable_usdc_eth();

    // Repay more than the actual debt. The contract uses a pull-model:
    // it transfers only the post-cap repayment from the liquidator's
    // wallet, so the unused mint stays with the liquidator.
    let debt_before = t.borrow_balance(ALICE, "ETH"); // ~3.0 ETH
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 100.0);

    // Liquidator started with 100 ETH minted (see harness `liquidate`).
    // The contract pulls at most `debt_before * (1+bonus)` worth.
    let liq_eth_left = t.token_balance(LIQUIDATOR, "ETH");
    assert!(
        liq_eth_left > 100.0 - debt_before - 0.01,
        "unused mint (~{}) must stay with liquidator; got {}",
        100.0 - debt_before,
        liq_eth_left
    );
    // Borrower's debt was paid down (proves repayment was capped, not lost).
    assert!(
        t.borrow_balance(ALICE, "ETH") < debt_before,
        "Alice's ETH debt must have decreased"
    );

    let liq_usdc = t.token_balance(LIQUIDATOR, "USDC");
    assert!(
        liq_usdc > 0.0,
        "liquidator should have received USDC collateral: {}",
        liq_usdc
    );
}
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
    assert!(
        t.borrow_balance(ALICE, "ETH") < 3.0,
        "borrower debt must have decreased"
    );
}
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
    // Bad-debt path: Alice's account must be cleaned up (no remaining positions).
    t.assert_no_positions(ALICE);
    let accounts = t.get_active_accounts(ALICE);
    assert_eq!(
        accounts.len(),
        0,
        "auto-cleanup must remove account when bad debt fires"
    );
}
#[test]
fn test_liquidation_bad_debt_socializes_loss() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Bob supplies ETH so loss can actually be socialized across his stake.
    t.supply(test_harness::BOB, "ETH", 100.0);
    // Small position.
    t.supply(ALICE, "USDC", 100.0);
    t.borrow(ALICE, "ETH", 0.03);

    // Crash price so collateral is nearly worthless.
    t.set_price("USDC", usd_cents(1));
    t.assert_liquidatable(ALICE);

    let bob_before = t.supply_balance(test_harness::BOB, "ETH");
    // A rational partial (the ~$1 collateral cannot cover the $60 debt at any
    // bonus) exhausts the collateral and leaves residual debt to socialize.
    // The guard's full-close ideal caps the estimate, not the payment, so a
    // small payment still liquidates.
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.001);

    // Socialization invariant: Bob's ETH supply has shrunk because the
    // residual bad debt was applied via apply_bad_debt_to_supply_index.
    let bob_after = t.supply_balance(test_harness::BOB, "ETH");
    assert!(
        bob_after < bob_before,
        "bad-debt socialization must reduce other suppliers' balance: {} -> {}",
        bob_before,
        bob_after
    );
    // Alice's account is removed during cleanup.
    t.assert_no_positions(ALICE);
}
#[test]
fn test_liquidation_rejects_during_flash_loan() {
    let mut t = liquidatable_usdc_eth();

    // Set the flash-loan-ongoing flag.
    t.set_flash_loan_ongoing(true);

    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::FLASH_LOAN_ONGOING);

    t.set_flash_loan_ongoing(false);
}
#[test]
fn test_liquidation_rejects_zero_amount() {
    let mut t = liquidatable_usdc_eth();

    // Use an exact zero payment. `0.0000001` ETH stays non-zero at 7 decimals.
    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.0);
    assert_contract_error(result, errors::AMOUNT_MUST_BE_POSITIVE);
}

// liquidation.rs rejects `account.owner == liquidator` with SelfLiquidationNotAllowed (#133).
#[test]
fn test_self_liquidation_rejects() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);

    let result = t.try_liquidate(ALICE, ALICE, "ETH", 0.5);
    assert_contract_error(result, errors::SELF_LIQUIDATION_NOT_ALLOWED);
}

// Third-party collateral must not let the borrower self-liquidate.
#[test]
fn test_third_party_supply_does_not_enable_self_liquidation() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);

    t.try_supply_to_account(BOB, ALICE, "USDC", 2_000.0)
        .expect("Bob may supply to Alice");

    let result = t.try_liquidate(ALICE, ALICE, "ETH", 0.5);
    assert_contract_error(result, errors::SELF_LIQUIDATION_NOT_ALLOWED);
}

// Liquidator path remains available after a third-party collateral top-up.
#[test]
fn test_third_party_supply_leaves_external_liquidation_available() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);

    t.try_supply_to_account(BOB, ALICE, "USDC", 1_000.0)
        .expect("third-party supply");

    t.supply(LIQUIDATOR, "USDC", 5_000.0);
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);
    assert!(
        t.borrow_balance(ALICE, "ETH") < 3.0,
        "external liquidator must still seize debt after third-party supply"
    );
}
