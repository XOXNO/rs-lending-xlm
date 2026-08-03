use super::{enable_dual_source, setup};
use test_harness::{assert_contract_error, errors, usd, usd_cents, ALICE, LIQUIDATOR};

#[test]
fn test_safe_price_allows_all_operations() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    t.set_safe_price("USDC", usd(1));
    t.set_safe_price("ETH", usd(2000));

    t.supply(ALICE, "USDC", 100_000.0);
    t.assert_supply_near(ALICE, "USDC", 100_000.0, 1.0);

    t.borrow(ALICE, "ETH", 10.0);
    t.assert_borrow_near(ALICE, "ETH", 10.0, 0.01);

    t.repay(ALICE, "ETH", 1.0);
    t.assert_borrow_near(ALICE, "ETH", 9.0, 0.01);

    t.withdraw(ALICE, "USDC", 1_000.0);
    t.assert_supply_near(ALICE, "USDC", 99_000.0, 1.0);
    t.assert_healthy(ALICE);
}

#[test]
fn test_single_tolerance_allows_risk_decreasing() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    t.set_safe_price("USDC", usd_cents(103));
    t.set_safe_price("ETH", usd(2000));

    t.supply(ALICE, "USDC", 100_000.0);
    t.assert_supply_near(ALICE, "USDC", 100_000.0, 1.0);

    t.borrow(ALICE, "ETH", 10.0);
    t.assert_borrow_near(ALICE, "ETH", 10.0, 0.01);
    t.assert_healthy(ALICE);

    t.repay(ALICE, "ETH", 1.0);
    t.assert_borrow_near(ALICE, "ETH", 9.0, 0.01);
}

#[test]
fn test_single_tolerance_allows_borrow() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    t.set_safe_price("USDC", usd_cents(103));
    t.set_safe_price("ETH", usd(2000));

    t.supply(ALICE, "USDC", 100_000.0);

    t.try_borrow(ALICE, "ETH", 10.0)
        .expect("borrow should work within single tolerance");
    t.assert_borrow_near(ALICE, "ETH", 10.0, 0.01);
    let eth_wallet = t.token_balance(ALICE, "ETH");
    assert!(
        eth_wallet > 9.99,
        "ETH wallet should be ~10, got {}",
        eth_wallet
    );
}

#[test]
fn test_unsafe_price_allows_supply() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");

    t.set_safe_price("USDC", usd_cents(110));

    t.supply(ALICE, "USDC", 10_000.0);
    t.assert_supply_near(ALICE, "USDC", 10_000.0, 1.0);
}

#[test]
fn test_unsafe_price_allows_repay() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    t.set_safe_price("USDC", usd(1));
    t.set_safe_price("ETH", usd(2000));

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    t.set_safe_price("ETH", usd(2200));

    let debt_before = t.borrow_balance(ALICE, "ETH");
    t.repay(ALICE, "ETH", 1.0);
    let debt_after = t.borrow_balance(ALICE, "ETH");
    assert!(
        debt_before - debt_after >= 0.99,
        "repay under unsafe price must reduce debt by ~1 ETH: before={}, after={}",
        debt_before,
        debt_after
    );
}

#[test]
fn test_unsafe_price_blocks_borrow() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    t.set_safe_price("USDC", usd(1));
    t.set_safe_price("ETH", usd(2000));

    t.supply(ALICE, "USDC", 100_000.0);

    t.set_safe_price("USDC", usd_cents(110));

    let result = t.try_borrow(ALICE, "ETH", 10.0);
    assert_contract_error(result, errors::UNSAFE_PRICE);
}

#[test]
fn test_unsafe_price_blocks_borrow_debt_asset() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    t.set_safe_price("USDC", usd(1));
    t.set_safe_price("ETH", usd(2000));

    t.supply(ALICE, "USDC", 100_000.0);

    t.set_safe_price("ETH", usd(2200));

    let result = t.try_borrow(ALICE, "ETH", 10.0);
    assert_contract_error(result, errors::UNSAFE_PRICE);
}

#[test]
fn test_unsafe_price_blocks_withdraw_with_borrows() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    t.set_safe_price("USDC", usd(1));
    t.set_safe_price("ETH", usd(2000));

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    t.set_safe_price("USDC", usd_cents(110));

    let result = t.try_withdraw(ALICE, "USDC", 1_000.0);
    assert_contract_error(result, errors::UNSAFE_PRICE);
}

#[test]
fn withdraw_succeeds_under_oracle_deviation_when_no_debt() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    t.set_safe_price("USDC", usd(1));
    t.set_safe_price("ETH", usd(2000));

    t.supply(ALICE, "USDC", 100_000.0);

    t.set_safe_price("USDC", usd_cents(110));

    let wallet_before = t.token_balance(ALICE, "USDC");
    t.try_withdraw(ALICE, "USDC", 1_000.0)
        .expect("withdraw should succeed under oracle deviation when account has no debt");
    t.assert_supply_near(ALICE, "USDC", 99_000.0, 1.0);
    let wallet_after = t.token_balance(ALICE, "USDC");
    assert!(
        wallet_after - wallet_before > 999.0,
        "wallet should grow by ~1000: before={}, after={}",
        wallet_before,
        wallet_after
    );
}

#[test]
fn withdraw_blocked_under_oracle_deviation_when_debt_exists() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    t.set_safe_price("USDC", usd(1));
    t.set_safe_price("ETH", usd(2000));

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    t.set_safe_price("USDC", usd_cents(110));

    let err = t
        .try_withdraw(ALICE, "USDC", 1_000.0)
        .expect_err("withdraw with borrows must fail under oracle deviation");

    let expected = soroban_sdk::Error::from_contract_error(205);
    assert_eq!(
        err, expected,
        "expected UnsafePriceNotAllowed (205), got {:?}",
        err
    );
}

#[test]
fn test_unsafe_price_blocks_liquidation() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    t.set_safe_price("USDC", usd(1));
    t.set_safe_price("ETH", usd(2000));

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 30.0);

    t.set_price("ETH", usd(3500));
    t.set_safe_price("ETH", usd(3500));

    assert!(t.can_be_liquidated(ALICE), "Alice should be liquidatable");

    t.supply(LIQUIDATOR, "ETH", 5.0);

    t.set_safe_price("USDC", usd_cents(110));

    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::UNSAFE_PRICE);
}
