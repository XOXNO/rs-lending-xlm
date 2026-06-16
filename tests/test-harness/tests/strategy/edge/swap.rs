use super::*;

#[test]
fn test_swap_debt_refund_only_uses_strategy_excess() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 0.5);

    let eth_market = t.resolve_market("ETH");
    let eth_client = token::Client::new(&t.env, &eth_market.asset);
    eth_market
        .token_admin
        .mint(&t.controller_address(), &50_0000000i128);

    t.fund_router("ETH", 1.0);
    // swap_debt borrows 0.005 WBTC (7 decimals = raw 50_000) minus 9bps fee.
    let steps = build_aggregator_swap(&t, "WBTC", "ETH", apply_flash_fee(50_000), 1_0000000);

    let alice_eth_before = t.token_balance(ALICE, "ETH");
    t.swap_debt(ALICE, "ETH", 0.005, "WBTC", &steps);
    let alice_eth_after = t.token_balance(ALICE, "ETH");
    let controller_eth_after = eth_client.balance(&t.controller_address());

    assert!(
        (alice_eth_after - alice_eth_before - 0.5).abs() < 0.01,
        "caller should only receive the 0.5 ETH overpayment, before={}, after={}",
        alice_eth_before,
        alice_eth_after
    );
    assert_eq!(
        controller_eth_after, 50_0000000i128,
        "unrelated controller ETH balance must not be swept to the caller"
    );
}
// Mutate stored collateral params in test-only setup so the final HF guard is
// stricter than the borrow-side LTV check.

#[test]
fn test_swap_debt_health_factor_guard_after_swap() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    let usdc = t.resolve_asset("USDC");
    t.env.as_contract(&t.controller_address(), || {
        let mut market: MarketConfig = t
            .env
            .storage()
            .persistent()
            .get(&ControllerKey::Market(usdc.clone()))
            .expect("USDC market should exist");
        market.asset_config.loan_to_value_bps = 9000;
        market.asset_config.liquidation_threshold_bps = 5000;
        t.env
            .storage()
            .persistent()
            .set(&ControllerKey::Market(usdc.clone()), &market);
    });

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 5.0);

    t.fund_router("ETH", 5.0);
    // swap_debt borrows 1.0 WBTC (7 decimals = raw 10_000_000) minus 9bps fee.
    let steps = build_aggregator_swap(&t, "WBTC", "ETH", apply_flash_fee(10_000_000), 5_0000000);
    let result = t.try_swap_debt(ALICE, "ETH", 1.0, "WBTC", &steps);

    assert_contract_error(result, errors::INSUFFICIENT_COLLATERAL);
}

#[test]
fn test_swap_debt_empty_swap_payload_rolls_back_new_debt() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let eth_debt_before = t.borrow_balance_raw(ALICE, "ETH");
    let wbtc_debt_before = t.borrow_balance_raw(ALICE, "WBTC");
    let empty_steps = Bytes::new(&t.env);

    let result = t.try_swap_debt(ALICE, "ETH", 0.005, "WBTC", &empty_steps);
    assert_contract_error(result, errors::INVALID_PAYMENTS);

    assert_eq!(
        t.borrow_balance_raw(ALICE, "ETH"),
        eth_debt_before,
        "existing ETH debt must roll back unchanged after an invalid route"
    );
    assert_eq!(
        t.borrow_balance_raw(ALICE, "WBTC"),
        wbtc_debt_before,
        "flash-opened WBTC debt must not survive empty route rejection"
    );
}

#[test]
fn test_swap_debt_closes_existing_debt_even_if_existing_asset_disabled() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let eth = t.resolve_asset("ETH");
    t.env.as_contract(&t.controller_address(), || {
        let mut market: MarketConfig = t
            .env
            .storage()
            .persistent()
            .get(&ControllerKey::Market(eth.clone()))
            .expect("ETH market should exist");
        market.asset_config.is_borrowable = false;
        t.env
            .storage()
            .persistent()
            .set(&ControllerKey::Market(eth.clone()), &market);
    });

    t.fund_router("ETH", 1.0);
    let steps = build_aggregator_swap(&t, "WBTC", "ETH", apply_flash_fee(50_000), 1_0000000);
    t.swap_debt(ALICE, "ETH", 0.005, "WBTC", &steps);

    assert_eq!(
        t.borrow_balance_raw(ALICE, "ETH"),
        0,
        "swap_debt should fully close the disabled existing debt asset"
    );
    assert!(
        t.borrow_balance_raw(ALICE, "WBTC") > 0,
        "new WBTC debt should remain after the rotation"
    );
}
// Swap debt edge cases

#[test]
fn test_swap_debt_rejects_when_paused() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    t.pause();

    let steps = build_swap_steps(&t, "WBTC", "ETH", 1_0000000);
    let result = t.try_swap_debt(ALICE, "ETH", 1.0, "WBTC", &steps);
    assert_contract_error(result, errors::CONTRACT_PAUSED);
}

#[test]
fn test_swap_debt_rejects_during_flash_loan() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    t.set_flash_loan_ongoing(true);

    let steps = build_swap_steps(&t, "WBTC", "ETH", 1_0000000);
    let result = t.try_swap_debt(ALICE, "ETH", 1.0, "WBTC", &steps);
    assert_contract_error(result, errors::FLASH_LOAN_ONGOING);
}
// The destination collateral leg must inherit the account's active eMode
// parameters, not the market's base parameters.

#[test]
fn test_swap_collateral_applies_emode_params_to_destination_position() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .with_emode_asset(1, "USDT", true, true)
        .build();

    let account_id = t.create_emode_account(ALICE, 1);
    t.supply_to(ALICE, account_id, "USDC", 5_000.0);

    t.fund_router("USDT", 1_000.0);
    // swap_collateral withdraws 1_000 USDC (raw 10_000_000_000); no flash fee.
    let steps = build_aggregator_swap(&t, "USDC", "USDT", 10_000_000_000, 10_000_000_000);
    t.swap_collateral(ALICE, "USDC", 1_000.0, "USDT", &steps);

    let (ltv, threshold) = supply_position_params(&t, account_id, "USDT");
    assert_eq!(ltv, 9700, "destination collateral should use eMode LTV");
    assert_eq!(
        threshold, 9800,
        "destination collateral should use eMode liquidation threshold"
    );
}

#[test]
fn test_swap_collateral_merges_existing_destination_and_removes_source() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 1_000.0);
    t.supply(ALICE, "ETH", 2.0);

    let eth_supply_before = t.supply_balance_raw(ALICE, "ETH");
    t.fund_router("ETH", 5.0);
    let steps = build_aggregator_swap(&t, "USDC", "ETH", 10_000_000_000, 5_0000000);

    t.swap_collateral(ALICE, "USDC", 1_000.0, "ETH", &steps);

    assert_eq!(
        t.supply_balance_raw(ALICE, "USDC"),
        0,
        "full source collateral swap should remove the old USDC position"
    );
    assert_eq!(
        t.supply_balance_raw(ALICE, "ETH"),
        eth_supply_before + 5_0000000,
        "destination ETH collateral should merge with the existing position"
    );
}
// New debt asset is_borrowable=false: must reject before the swap.

#[test]
fn test_swap_debt_non_borrowable_new_debt() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_market_config("WBTC", |c| {
            c.is_borrowable = false;
        })
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let steps = build_swap_steps(&t, "WBTC", "ETH", 1_0000000);
    let result = t.try_swap_debt(ALICE, "ETH", 1.0, "WBTC", &steps);
    assert_contract_error(result, errors::ASSET_NOT_BORROWABLE);
}
// The new debt asset has a borrow cap that would be exceeded. The cap check
// runs after pool.borrow(), which runs before the swap.

#[test]
fn test_swap_debt_borrow_cap_new_debt() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_market_config("WBTC", |c| {
            // Set a very low borrow cap: 1 unit (0.0000001 WBTC).
            c.borrow_cap = 1;
        })
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // Swap ETH debt to WBTC: the WBTC borrow cap is tiny.
    let steps = build_swap_steps(&t, "WBTC", "ETH", 1_00000000);
    let result = t.try_swap_debt(ALICE, "ETH", 1.0, "WBTC", &steps);
    assert_contract_error(result, errors::BORROW_CAP_REACHED);
}
// E-mode account; the new debt asset is not in the e-mode category.

#[test]
fn test_swap_debt_emode_wrong_category() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_market(eth_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .with_emode_asset(1, "USDT", true, true)
        // ETH not in e-mode
        .build();

    // Create an e-mode account, supply USDC, borrow USDT (both in e-mode).
    t.create_emode_account(ALICE, 1);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 5_000.0);

    // Try to swap USDT debt to ETH: ETH is not in the e-mode category.
    let steps = build_swap_steps(&t, "ETH", "USDT", 5000_0000000);
    let result = t.try_swap_debt(ALICE, "USDT", 5_000.0, "ETH", &steps);
    assert_contract_error(result, errors::EMODE_CATEGORY_NOT_FOUND);
}
// Swap collateral edge cases

#[test]
fn test_swap_collateral_rejects_when_paused() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    t.pause();

    let steps = build_swap_steps(&t, "USDC", "ETH", 5_0000000);
    let result = t.try_swap_collateral(ALICE, "USDC", 1000.0, "ETH", &steps);
    assert_contract_error(result, errors::CONTRACT_PAUSED);
}

#[test]
fn test_swap_collateral_rejects_during_flash_loan() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    t.set_flash_loan_ongoing(true);

    let steps = build_swap_steps(&t, "USDC", "ETH", 5_0000000);
    let result = t.try_swap_collateral(ALICE, "USDC", 1000.0, "ETH", &steps);
    assert_contract_error(result, errors::FLASH_LOAN_ONGOING);
}
// New collateral is_collateralizable=false.

#[test]
fn test_swap_collateral_non_collateralizable() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_market_config("WBTC", |c| {
            c.is_collateralizable = false;
        })
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // Try to swap USDC collateral to non-collateralizable WBTC
    let steps = build_swap_steps(&t, "USDC", "WBTC", 1_00000000);
    let result = t.try_swap_collateral(ALICE, "USDC", 1000.0, "WBTC", &steps);
    assert_contract_error(result, errors::NOT_COLLATERAL);
}
// Fund the router so the flow reaches the post-deposit cap check.

#[test]
fn test_swap_collateral_rejects_supply_cap_after_deposit() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_market_config("WBTC", |c| {
            c.supply_cap = 1; // extremely low: 1 unit (0.0000001 WBTC).
        })
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    t.fund_router_raw("WBTC", 1_00000000i128);
    // swap_collateral withdraws 1_000 USDC (raw 10_000_000_000); no flash fee.
    let steps = build_aggregator_swap(&t, "USDC", "WBTC", 10_000_000_000, 1_00000000);
    let result = t.try_swap_collateral(ALICE, "USDC", 1_000.0, "WBTC", &steps);

    assert_contract_error(result, errors::SUPPLY_CAP_REACHED);
}
