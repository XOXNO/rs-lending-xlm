use controller::constants::WAD;
use test_harness::{
    assert_contract_error, days, errors, eth_preset, usdc_preset, usdt_stable_preset, wbtc_preset,
    LendingTest, PositionType, ALICE, BOB, LIQUIDATOR,
};
// 1. test_hf_above_one_after_every_borrow

#[test]
fn test_hf_above_one_after_every_borrow() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);

    // Borrow near the LTV limit (~75% of $100k = $75k, or 37.5 ETH at $2k/ETH),
    // incrementally, and require HF >= 1.0 after each step.
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
// 2. test_hf_above_one_after_every_withdraw

#[test]
fn test_hf_above_one_after_every_withdraw() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 5.0); // ~$10k debt

    // Withdraw incrementally.
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
// 3. test_hf_below_one_required_for_liquidation

#[test]
fn test_hf_below_one_required_for_liquidation() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    // Healthy -- liquidation must fail.
    t.assert_healthy(ALICE);
    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::HEALTH_FACTOR_TOO_HIGH);
}
// 4. test_ltv_less_than_threshold_always

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
// 5. test_supply_index_monotonically_increasing

#[test]
fn test_supply_index_monotonically_increasing() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Supply + borrow to generate interest (supply index grows when borrows exist).
    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(BOB, "ETH", 100.0);
    t.borrow(ALICE, "ETH", 10.0);

    let mut prev_balance = t.supply_balance(BOB, "ETH");
    let initial_balance = prev_balance;

    // Check supply balance grows over time (proxy for the supply index).
    // Require strict inequality to detect stalled accrual:
    // a `>=` check would silently accept `current == prev` forever (e.g. if
    // reserve_factor reached 100% or the rate fell to zero).
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

    // After 4 weeks of ALICE borrowing 10 ETH against BOB's 100 ETH supply,
    // total accrual must exceed dust. This catches "index inches up by 1
    // ulp per week" drift.
    let total_growth = prev_balance - initial_balance;
    assert!(
        total_growth > 0.0001,
        "supply balance must grow by more than dust over 28 days, got {}",
        total_growth
    );
}
// 6. test_borrow_index_monotonically_increasing

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

    // Strict inequality detects a frozen borrow index that would otherwise
    // leave accrued interest uncharged.
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
// 7. test_position_limits_enforced

#[test]
fn test_position_limits_enforced() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_market(usdt_stable_preset())
        .with_position_limits(2, 2)
        .build();

    // Supply to 2 markets (at the limit).
    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 1.0);

    // The third supply must reject with the specific POSITION_LIMIT_EXCEEDED
    // error. A bare `is_err()` check accepts any failure, including pause,
    // stale oracle, or internal errors.
    let result = t.try_supply(ALICE, "WBTC", 0.01);
    assert_contract_error(result, errors::POSITION_LIMIT_EXCEEDED);
}
// 8. test_total_supply_matches_pool_balance

#[test]
fn test_total_supply_matches_pool_balance() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 50_000.0);
    t.supply(BOB, "USDC", 30_000.0);

    // Total supply across users should match.
    let alice_supply = t.supply_balance(ALICE, "USDC");
    let bob_supply = t.supply_balance(BOB, "USDC");
    let total_user_supply = alice_supply + bob_supply;

    // Should land near 80k.
    assert!(
        (total_user_supply - 80_000.0).abs() < 10.0,
        "total supply should be ~80k, got {}",
        total_user_supply
    );

    // Invariant: pool token balance >= total user supply.
    // The pool was seeded with 1M initial liquidity, then users supplied 80k
    // more, so the pool contract holds at least the user-supplied amount.
    let pool_balance = t.pool_reserves("USDC");
    assert!(
        pool_balance >= total_user_supply,
        "pool reserves ({}) should be >= total user supply ({})",
        pool_balance,
        total_user_supply
    );
}
// 10. test_full_lifecycle_supply_borrow_repay_withdraw

#[test]
fn test_full_lifecycle_supply_borrow_repay_withdraw() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // 1. Supply.
    t.supply(ALICE, "USDC", 100_000.0);
    t.assert_position_exists(ALICE, "USDC", PositionType::Supply);
    t.assert_supply_near(ALICE, "USDC", 100_000.0, 1.0);

    // 2. Borrow.
    t.borrow(ALICE, "ETH", 5.0);
    t.assert_position_exists(ALICE, "ETH", PositionType::Borrow);
    t.assert_healthy(ALICE);

    // 3. Advance time (interest accrues).
    t.advance_and_sync(days(30));
    let debt_with_interest = t.borrow_balance(ALICE, "ETH");
    assert!(debt_with_interest > 5.0, "debt should include interest");

    // 4. Repay full debt (with extra for interest).
    t.repay(ALICE, "ETH", debt_with_interest + 0.1);

    // 5. Borrow balance should be ~0.
    let remaining_debt = t.borrow_balance(ALICE, "ETH");
    assert!(
        remaining_debt < 0.001,
        "debt should be ~0 after full repay, got {}",
        remaining_debt
    );

    // 6. Withdraw all collateral.
    t.withdraw_all(ALICE, "USDC");

    // 7. Supply balance should be ~0.
    let remaining_supply = t.supply_balance(ALICE, "USDC");
    assert!(
        remaining_supply < 0.01,
        "supply should be ~0 after full withdraw, got {}",
        remaining_supply
    );

    // 8. Remove the account if it still exists. The protocol may have
    // already cleaned it up after the final position close.
    let _ = t.try_remove_account(ALICE);
}
