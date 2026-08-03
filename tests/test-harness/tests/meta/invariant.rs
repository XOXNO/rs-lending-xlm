use controller::constants::WAD;
use test_harness::{
    assert_contract_error, days, errors, eth_preset, usdc_preset, usdt_stable_preset, wbtc_preset,
    LendingTest, PositionType, ALICE, BOB, LIQUIDATOR,
};
#[test]
fn test_hf_above_one_after_every_borrow() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);

    for i in 1..=10 {
        t.borrow(ALICE, "ETH", 3.0);
        let hf = t.health_factor_raw(ALICE);
        assert!(
            hf >= WAD,
            "HF should be >= 1.0 after borrow #{}: HF = {}",
            i,
            hf as f64 / WAD as f64
        );
    }
}
#[test]
fn test_hf_above_one_after_every_withdraw() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 5.0);

    for i in 1..=5 {
        t.withdraw(ALICE, "USDC", 10_000.0);
        let hf = t.health_factor_raw(ALICE);
        assert!(
            hf >= WAD,
            "HF should be >= 1.0 after withdraw #{}: HF = {}",
            i,
            hf as f64 / WAD as f64
        );
    }
}
#[test]
fn test_hf_below_one_required_for_liquidation() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    t.assert_healthy(ALICE);
    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::HEALTH_FACTOR_TOO_HIGH);
}
#[test]
fn test_ltv_less_than_threshold_always() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    for market in &["USDC", "ETH", "WBTC"] {
        let config = t.get_asset_config(market);
        assert!(
            config.loan_to_value < config.liquidation_threshold,
            "{}: LTV ({}) should be < threshold ({})",
            market,
            config.loan_to_value,
            config.liquidation_threshold
        );
    }
}
#[test]
fn test_supply_index_monotonically_increasing() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(BOB, "ETH", 100.0);
    t.borrow(ALICE, "ETH", 10.0);

    let mut prev_balance = t.supply_balance(BOB, "ETH");
    let initial_balance = prev_balance;

    for week in 1..=4 {
        t.advance_and_sync(days(7));
        let current_balance = t.supply_balance(BOB, "ETH");
        assert!(
            current_balance > prev_balance,
            "supply balance must STRICTLY increase week {}: prev={}, current={}",
            week,
            prev_balance,
            current_balance
        );
        prev_balance = current_balance;
    }

    let total_growth = prev_balance - initial_balance;
    assert!(
        total_growth > 0.0001,
        "supply balance must grow by more than dust over 28 days, got {}",
        total_growth
    );
}
#[test]
fn test_borrow_index_monotonically_increasing() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    let mut prev_debt = t.borrow_balance(ALICE, "ETH");
    let initial_debt = prev_debt;

    for week in 1..=4 {
        t.advance_and_sync(days(7));
        let current_debt = t.borrow_balance(ALICE, "ETH");
        assert!(
            current_debt > prev_debt,
            "borrow debt must STRICTLY increase week {}: prev={}, current={}",
            week,
            prev_debt,
            current_debt
        );
        prev_debt = current_debt;
    }

    let total_growth = prev_debt - initial_debt;
    assert!(
        total_growth > 0.0001,
        "borrow debt must grow by more than dust over 28 days, got {}",
        total_growth
    );
}
#[test]
fn test_position_limits_enforced() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_market(usdt_stable_preset())
        .with_position_limits(2, 2)
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 1.0);

    let result = t.try_supply(ALICE, "WBTC", 0.01);
    assert_contract_error(result, errors::POSITION_LIMIT_EXCEEDED);
}
#[test]
fn test_total_supply_matches_pool_balance() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 50_000.0);
    t.supply(BOB, "USDC", 30_000.0);

    let alice_supply = t.supply_balance(ALICE, "USDC");
    let bob_supply = t.supply_balance(BOB, "USDC");
    let total_user_supply = alice_supply + bob_supply;

    assert!(
        (total_user_supply - 80_000.0).abs() < 10.0,
        "total supply should be ~80k, got {}",
        total_user_supply
    );

    let pool_balance = t.pool_reserves("USDC");
    assert!(
        pool_balance >= total_user_supply,
        "pool reserves ({}) should be >= total user supply ({})",
        pool_balance,
        total_user_supply
    );
}
#[test]
fn test_full_lifecycle_supply_borrow_repay_withdraw() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.assert_position_exists(ALICE, "USDC", PositionType::Supply);
    t.assert_supply_near(ALICE, "USDC", 100_000.0, 1.0);

    t.borrow(ALICE, "ETH", 5.0);
    t.assert_position_exists(ALICE, "ETH", PositionType::Borrow);
    t.assert_healthy(ALICE);

    t.advance_and_sync(days(30));
    let debt_with_interest = t.borrow_balance(ALICE, "ETH");
    assert!(debt_with_interest > 5.0, "debt should include interest");

    t.repay(ALICE, "ETH", debt_with_interest + 0.1);

    let remaining_debt = t.borrow_balance(ALICE, "ETH");
    assert!(
        remaining_debt < 0.001,
        "debt should be ~0 after full repay, got {}",
        remaining_debt
    );

    t.withdraw_all(ALICE, "USDC");

    let remaining_supply = t.supply_balance(ALICE, "USDC");
    assert!(
        remaining_supply < 0.01,
        "supply should be ~0 after full withdraw, got {}",
        remaining_supply
    );

    let _ = t.try_remove_account(ALICE);
}
