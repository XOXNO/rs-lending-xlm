extern crate std;

use test_harness::{
    days, eth_preset, usd_cents, usdc_preset, usdt_stable_preset, LendingTest, PositionType, ALICE,
    LIQUIDATOR, STABLECOIN_EMODE,
};

// ---------------------------------------------------------------------------
// 1. test_supply_creates_position
// ---------------------------------------------------------------------------

#[test]
fn test_supply_creates_position() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 10_000.0);

    // The position must exist.
    t.assert_position_exists(ALICE, "USDC", PositionType::Supply);

    // Wallet balance must be 0 (all tokens moved to the protocol).
    let wallet = t.token_balance(ALICE, "USDC");
    assert!(
        wallet < 0.01,
        "wallet should be ~0 after supply, got {}",
        wallet
    );

    // Supply balance must be ~10_000.
    t.assert_supply_near(ALICE, "USDC", 10_000.0, 1.0);
}

// ---------------------------------------------------------------------------
// 2. test_supply_and_borrow
// ---------------------------------------------------------------------------

#[test]
fn test_supply_and_borrow() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Supply 10k USDC as collateral.
    t.supply(ALICE, "USDC", 10_000.0);

    // Borrow 1 ETH (~$2000, well within 75% LTV of $10k = $7500).
    t.borrow(ALICE, "ETH", 1.0);

    t.assert_position_exists(ALICE, "USDC", PositionType::Supply);
    t.assert_position_exists(ALICE, "ETH", PositionType::Borrow);
    t.assert_healthy(ALICE);

    // Verify the borrow balance is ~1 ETH.
    t.assert_borrow_near(ALICE, "ETH", 1.0, 0.01);
}

// ---------------------------------------------------------------------------
// 3. test_liquidation_after_price_drop
// ---------------------------------------------------------------------------

#[test]
fn test_liquidation_after_price_drop() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Alice supplies 10k USDC as collateral.
    t.supply(ALICE, "USDC", 10_000.0);

    // Borrow 3 ETH (~$6000, near the 75% LTV limit of $7500).
    t.borrow(ALICE, "ETH", 3.0);
    t.assert_healthy(ALICE);

    // Drop USDC price to $0.50: collateral value becomes $5000.
    // liquidation_threshold = 80% => weighted collateral = $4000.
    // debt = $6000 => HF = 4000/6000 ~ 0.67 => liquidatable.
    t.set_price("USDC", usd_cents(50));

    t.assert_liquidatable(ALICE);

    // The liquidator repays part of Alice's ETH debt.
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);

    // The liquidator must have received USDC collateral.
    let liq_usdc_after = t.token_balance(LIQUIDATOR, "USDC");
    assert!(
        liq_usdc_after > 0.0,
        "liquidator should have received collateral, got {}",
        liq_usdc_after
    );
}

// ---------------------------------------------------------------------------
// 4. test_interest_accrues
// ---------------------------------------------------------------------------

#[test]
fn test_interest_accrues() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let debt_before = t.borrow_balance(ALICE, "ETH");

    // Advance 365 days and sync indexes.
    t.advance_and_sync(days(365));

    let debt_after = t.borrow_balance(ALICE, "ETH");
    assert!(
        debt_after > debt_before,
        "debt should have grown after 1 year: before={}, after={}",
        debt_before,
        debt_after
    );

    // Interest must be meaningful (at least 0.1% on 1 ETH).
    let interest = debt_after - debt_before;
    assert!(
        interest > 0.001,
        "interest should be non-trivial, got {}",
        interest
    );
}

// ---------------------------------------------------------------------------
// 5. test_withdraw_and_repay
// ---------------------------------------------------------------------------

#[test]
fn test_withdraw_and_repay() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Supply 100k USDC, borrow 1 ETH.
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // Partial withdraw: take back 10k USDC.
    t.withdraw(ALICE, "USDC", 10_000.0);
    let wallet_after_withdraw = t.token_balance(ALICE, "USDC");
    assert!(
        wallet_after_withdraw > 9_999.0,
        "should have ~10k USDC in wallet after withdraw, got {}",
        wallet_after_withdraw
    );

    // Supply balance must be ~90k.
    t.assert_supply_near(ALICE, "USDC", 90_000.0, 1.0);

    // Repay the borrow in full.
    t.repay(ALICE, "ETH", 1.0);

    // Borrow balance must be ~0.
    let borrow_after = t.borrow_balance(ALICE, "ETH");
    assert!(
        borrow_after < 0.01,
        "borrow should be ~0 after full repay, got {}",
        borrow_after
    );

    t.assert_healthy(ALICE);
}

// ---------------------------------------------------------------------------
// 6. test_emode_higher_ltv
// ---------------------------------------------------------------------------

#[test]
fn test_emode_higher_ltv() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .with_emode_asset(1, "USDT", true, true)
        .build();

    // Create an e-mode account for Alice.
    t.create_emode_account(ALICE, 1);

    // Supply 10k USDC.
    t.supply(ALICE, "USDC", 10_000.0);

    // Borrow at 95% LTV = $9500 USDT. E-mode LTV is 97%, so 95% is safe.
    t.borrow(ALICE, "USDT", 9_500.0);

    t.assert_healthy(ALICE);

    // Verify the health factor is above 1.0 but relatively tight.
    let hf = t.health_factor(ALICE);
    assert!(
        (1.0..1.10).contains(&hf),
        "e-mode HF should be tight but healthy, got {}",
        hf
    );
}

// ---------------------------------------------------------------------------
// 7. test_revenue_accrues_over_time
// ---------------------------------------------------------------------------

#[test]
fn test_revenue_accrues_over_time() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Set up: supply and borrow to generate interest.
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    // Advance time to accrue interest, which generates protocol revenue.
    t.advance_and_sync(days(30));

    let revenue_before = t.snapshot_revenue("ETH");

    // Advance more time so more interest accrues.
    t.advance_and_sync(days(30));

    let revenue_after = t.snapshot_revenue("ETH");
    assert!(
        revenue_after > revenue_before,
        "revenue should increase over time: before={}, after={}",
        revenue_before,
        revenue_after
    );
}
