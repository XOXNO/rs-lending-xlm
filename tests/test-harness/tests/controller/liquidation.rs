use test_harness::{
    assert_contract_error, errors, eth_preset, liquidatable_usdc_eth, usd_cents, usdc_preset,
    LendingTest, ALICE, BOB, LIQUIDATOR,
};
#[test]
fn test_liquidation_basic_proportional() {
    let mut t = liquidatable_usdc_eth();

    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);

    let liq_usdc_after = t.token_balance(LIQUIDATOR, "USDC");
    assert!(
        liq_usdc_after > 0.0,
        "liquidator should have received USDC collateral, got {}",
        liq_usdc_after
    );

    let collateral_value_usd = liq_usdc_after * 0.50;
    let debt_paid_usd = 1.0 * 2000.0;
    assert!(
        collateral_value_usd > debt_paid_usd,
        "liquidator should profit from bonus: collateral ${:.2} > debt ${:.2}",
        collateral_value_usd,
        debt_paid_usd
    );

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

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);

    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);

    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);

    let liq_usdc = t.token_balance(LIQUIDATOR, "USDC");
    assert!(
        liq_usdc > 0.0,
        "liquidator should have received USDC collateral"
    );

    assert!(t.borrow_balance(ALICE, "ETH") < 3.0);
    assert!(t.supply_balance(ALICE, "USDC") < 10_000.0);
    assert!(
        t.find_account_id(ALICE).is_some(),
        "partial liquidation must leave the account open"
    );
}
#[test]
fn test_liquidation_rejects_healthy_account() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.assert_healthy(ALICE);

    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);
    assert_contract_error(result, errors::HEALTH_FACTOR_TOO_HIGH);
}
#[test]
fn test_liquidation_allowed_when_paused() {
    let mut t = liquidatable_usdc_eth();
    t.pause();

    let debt_before = t.borrow_balance(ALICE, "ETH");
    let coll_before = t.supply_balance(ALICE, "USDC");
    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    assert!(
        result.is_ok(),
        "liquidation should remain available while paused"
    );
    assert!(
        t.borrow_balance(ALICE, "ETH") < debt_before,
        "paused liquidation must still reduce debt"
    );
    assert!(
        t.supply_balance(ALICE, "USDC") < coll_before,
        "paused liquidation must still seize collateral"
    );
    assert!(
        t.token_balance(LIQUIDATOR, "USDC") > 0.0,
        "liquidator must receive seized USDC while paused"
    );
}
#[test]
fn test_liquidation_dynamic_bonus_moderate() {
    let mut t = liquidatable_usdc_eth();

    t.set_price("USDC", usd_cents(71));
    t.assert_liquidatable(ALICE);
    let hf_before = t.health_factor(ALICE);

    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);

    let liq_usdc = t.token_balance(LIQUIDATOR, "USDC");

    let collateral_received_usd = liq_usdc * 0.71;

    assert!(
        collateral_received_usd > 2000.0,
        "liquidator should profit from bonus: received ${} of collateral for $2000 debt",
        collateral_received_usd
    );

    let bonus_rate = collateral_received_usd / 2000.0 - 1.0;
    assert!(
        bonus_rate > 0.10 && bonus_rate < 0.25,
        "moderate-HF bonus must be a mid-range HF-scaled value, got {:.4}",
        bonus_rate
    );

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

    t.set_price("USDC", usd_cents(25));
    t.assert_liquidatable(ALICE);

    let hf = t.health_factor(ALICE);
    assert!(hf < 0.5, "HF should be deeply underwater, got {}", hf);

    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);

    let liq_usdc = t.token_balance(LIQUIDATOR, "USDC");
    assert!(liq_usdc > 0.0, "liquidator should receive collateral");
    assert!(t.borrow_balance(ALICE, "ETH") < 3.0);

    let collateral_received_usd = liq_usdc * 0.25;
    let bonus_rate = collateral_received_usd / 2000.0 - 1.0;
    assert!(
        bonus_rate > 0.0 && bonus_rate < 0.10,
        "deep-underwater bonus must sit near the 5% base (fee-adjusted), got {:.4}",
        bonus_rate
    );
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
fn test_liquidation_sequential_partial_liquidations() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);

    t.set_price("USDC", usd_cents(30));
    t.assert_liquidatable(ALICE);

    let debt_before = t.borrow_balance(ALICE, "ETH");
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);
    let debt_after_first = t.borrow_balance(ALICE, "ETH");
    assert!(
        debt_after_first < debt_before,
        "1st liquidation must reduce debt"
    );

    // At 30c collateral the account stays deeply insolvent after a 0.5 ETH
    // repay; assert it so the second round can never silently not run.
    assert!(
        t.can_be_liquidated(ALICE),
        "fixture must remain liquidatable after the first partial round"
    );
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.3);
    assert!(
        t.borrow_balance(ALICE, "ETH") < debt_after_first,
        "2nd liquidation must reduce debt further"
    );

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

    let debt_before = t.borrow_balance(ALICE, "ETH");
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 100.0);

    let liq_eth_left = t.token_balance(LIQUIDATOR, "ETH");
    assert!(
        liq_eth_left > 100.0 - debt_before - 0.01,
        "unused mint (~{}) must stay with liquidator; got {}",
        100.0 - debt_before,
        liq_eth_left
    );

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

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);

    t.set_price("USDC", usd_cents(70));
    t.assert_liquidatable(ALICE);

    let hf_before = t.health_factor(ALICE);

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

    t.set_price("USDC", usd_cents(10));
    t.assert_liquidatable(ALICE);

    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);

    let usdc_received = t.token_balance(LIQUIDATOR, "USDC");
    let usdc_value = usdc_received * 0.10;
    let debt_paid = 0.5 * 2000.0;
    assert!(usdc_received > 0.0, "liquidator should receive collateral");
    let ratio = usdc_value / debt_paid;
    assert!(
        ratio <= 1.10,
        "toxic-band bonus must stay at/under the 5% base (+ tol): got {:.4}",
        ratio,
    );
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

    t.supply(ALICE, "USDC", 100.0);
    t.borrow(ALICE, "ETH", 0.03);

    t.set_price("USDC", usd_cents(5));
    t.assert_liquidatable(ALICE);

    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.03);

    let liq_usdc = t.token_balance(LIQUIDATOR, "USDC");
    assert!(
        liq_usdc > 0.0,
        "liquidator should have received USDC collateral: {}",
        liq_usdc
    );

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

    t.supply(test_harness::BOB, "ETH", 100.0);

    t.supply(ALICE, "USDC", 100.0);
    t.borrow(ALICE, "ETH", 0.03);

    t.set_price("USDC", usd_cents(1));
    t.assert_liquidatable(ALICE);

    let bob_before = t.supply_balance(test_harness::BOB, "ETH");

    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.001);

    let bob_after = t.supply_balance(test_harness::BOB, "ETH");
    assert!(
        bob_after < bob_before,
        "bad-debt socialization must reduce other suppliers' balance: {} -> {}",
        bob_before,
        bob_after
    );

    t.assert_no_positions(ALICE);
}
#[test]
fn test_liquidation_rejects_during_flash_loan() {
    let mut t = liquidatable_usdc_eth();

    t.set_flash_loan_ongoing(true);

    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::FLASH_LOAN_ONGOING);

    t.set_flash_loan_ongoing(false);
}
#[test]
fn test_liquidation_rejects_zero_amount() {
    let mut t = liquidatable_usdc_eth();

    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.0);
    assert_contract_error(result, errors::AMOUNT_MUST_BE_POSITIVE);
}

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
