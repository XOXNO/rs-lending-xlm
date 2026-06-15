use test_harness::{
    assert_contract_error, errors, eth_preset, usdc_preset, usdt_stable_preset, LendingTest, ALICE,
};

#[test]
fn test_small_supply_succeeds_without_per_asset_dust_gate() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 5.0);
    t.assert_supply_near(ALICE, "USDC", 5.0, 1.0);
}

#[test]
fn test_borrow_rejected_when_ltv_collateral_below_instance_floor() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // $4 LTV-weighted collateral (0.75 × $4 = $3) with $3 debt.
    t.supply(ALICE, "USDC", 4.0);
    let res = t.try_borrow(ALICE, "ETH", 0.0015);
    assert_contract_error(res, errors::MIN_BORROW_COLLATERAL_NOT_MET);
}

#[test]
fn test_borrow_succeeds_when_ltv_collateral_meets_instance_floor() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100.0);
    t.borrow(ALICE, "ETH", 0.01);
}

#[test]
fn test_withdraw_while_in_debt_rejected_when_ltv_collateral_falls_below_floor() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Keep debt small so LTV still passes while LTV-weighted collateral drops
    // below the $5 instance floor.
    t.supply(ALICE, "USDC", 12.0);
    t.borrow(ALICE, "ETH", 0.001);
    let res = t.try_withdraw(ALICE, "USDC", 9.0);
    assert_contract_error(res, errors::MIN_BORROW_COLLATERAL_NOT_MET);
}

#[test]
fn test_withdraw_while_debt_free_allows_small_residue() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 100.0);
    t.withdraw(ALICE, "USDC", 95.0);
    t.assert_supply_near(ALICE, "USDC", 5.0, 1.0);
}

#[test]
fn test_partial_repay_leaving_small_debt_succeeds() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 0.025);
    t.repay(ALICE, "ETH", 0.024);
}

#[test]
fn test_min_borrow_collateral_gate_disabled_when_floor_zero() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 4.0);
    t.borrow(ALICE, "ETH", 0.0015);
}

#[test]
fn test_borrow_not_blocked_by_unrelated_supply_price_crash() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(usdt_stable_preset())
        .build();

    t.supply_bulk(ALICE, &[("ETH", 0.5), ("USDC", 15.0)]);
    t.set_price("USDC", controller::constants::WAD / 2);
    let result = t.try_borrow(ALICE, "USDT", 50.0);
    assert!(
        result.is_ok(),
        "borrow should use aggregate LTV collateral: {result:?}"
    );
}
