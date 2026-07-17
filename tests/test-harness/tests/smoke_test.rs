extern crate std;

use test_harness::{
    apply_flash_fee, build_aggregator_swap, days, eth_preset, usd_cents, usdc_preset,
    usdt_stable_preset, LendingTest, PositionType, ALICE, BOB, LIQUIDATOR, STABLECOIN_SPOKE,
};
#[test]
fn test_supply_creates_position() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 10_000.0);

    t.assert_position_exists(ALICE, "USDC", PositionType::Supply);

    let wallet = t.token_balance(ALICE, "USDC");
    assert!(
        wallet < 0.01,
        "wallet should be ~0 after supply, got {}",
        wallet
    );

    t.assert_supply_near(ALICE, "USDC", 10_000.0, 1.0);
}

#[test]
fn test_supply_and_borrow() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 10_000.0);
    // 1 ETH ~$2000; within 75% LTV of $10k = $7500.
    t.borrow(ALICE, "ETH", 1.0);

    t.assert_position_exists(ALICE, "USDC", PositionType::Supply);
    t.assert_position_exists(ALICE, "ETH", PositionType::Borrow);
    t.assert_healthy(ALICE);

    t.assert_borrow_near(ALICE, "ETH", 1.0, 0.01);
}

#[test]
fn test_liquidation_after_price_drop() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 10_000.0);
    // 3 ETH ~$6000, near 75% LTV of $7500.
    t.borrow(ALICE, "ETH", 3.0);
    t.assert_healthy(ALICE);

    // USDC @ $0.50 → coll $5000; LT 80% → weighted $4000; debt $6000 → HF ~0.67.
    t.set_price("USDC", usd_cents(50));

    t.assert_liquidatable(ALICE);

    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);

    let liq_usdc_after = t.token_balance(LIQUIDATOR, "USDC");
    assert!(
        liq_usdc_after > 0.0,
        "liquidator should have received collateral, got {}",
        liq_usdc_after
    );
}

#[test]
fn test_interest_accrues() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let debt_before = t.borrow_balance(ALICE, "ETH");

    t.advance_and_sync(days(365));

    let debt_after = t.borrow_balance(ALICE, "ETH");
    assert!(
        debt_after > debt_before,
        "debt should have grown after 1 year: before={}, after={}",
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

#[test]
fn test_withdraw_and_repay() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    t.withdraw(ALICE, "USDC", 10_000.0);
    let wallet_after_withdraw = t.token_balance(ALICE, "USDC");
    assert!(
        wallet_after_withdraw > 9_999.0,
        "should have ~10k USDC in wallet after withdraw, got {}",
        wallet_after_withdraw
    );

    t.assert_supply_near(ALICE, "USDC", 90_000.0, 1.0);

    t.repay(ALICE, "ETH", 1.0);

    let borrow_after = t.borrow_balance(ALICE, "ETH");
    assert!(
        borrow_after < 0.01,
        "borrow should be ~0 after full repay, got {}",
        borrow_after
    );

    t.assert_healthy(ALICE);
}

#[test]
fn test_spoke_higher_ltv() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    // 95% LTV; spoke LTV 97% allows it.
    t.borrow(ALICE, "USDT", 9_500.0);

    t.assert_healthy(ALICE);

    let hf = t.health_factor(ALICE);
    assert!(
        (1.0..1.10).contains(&hf),
        "spoke HF should be tight but healthy, got {}",
        hf
    );
}

#[test]
fn test_revenue_accrues_over_time() {
    let mut t = LendingTest::new().standard_two_asset().build();

    // Seed ETH supply so empty-pool guard in add_protocol_revenue does not skip.
    t.supply(BOB, "ETH", 50.0);
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    t.advance_and_sync(days(30));

    let revenue_before = t.snapshot_revenue("ETH");

    t.advance_and_sync(days(30));

    let revenue_after = t.snapshot_revenue("ETH");
    assert!(
        revenue_after > revenue_before,
        "revenue should increase over time: before={}, after={}",
        revenue_before,
        revenue_after
    );
}

#[test]
fn test_multiply_smoke_creates_leveraged_position() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.fund_router("USDC", 3_000.0);
    let steps = build_aggregator_swap(
        &t,
        "ETH",
        "USDC",
        apply_flash_fee(10_000_000),
        30_000_000_000,
    );
    let account_id = t.multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        controller::types::PositionMode::Multiply,
        &steps,
    );
    assert!(account_id > 0, "multiply should create an account");
    t.assert_healthy(ALICE);
    assert!(
        t.supply_balance(ALICE, "USDC") > 1_000.0,
        "multiply should deposit swapped USDC collateral"
    );
}
