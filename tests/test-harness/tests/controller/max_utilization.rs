use controller::constants::RAY;
use test_harness::{
    assert_contract_error, days, errors, hub_asset, map_try_ok_unit, usdc_preset, wbtc_preset,
    HubAssetKey, LendingTest, MarketPreset, ALICE, BOB, CAROL,
};

#[test]
fn test_borrow_above_max_utilization_rejected() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .without_initial_liquidity()
        .with_market_params("USDC", |p| {
            p.max_utilization = RAY * 85 / 100;
        })
        .build();

    t.supply(ALICE, "USDC", 1_000.0);
    t.supply(BOB, "ETH", 10.0);

    t.borrow(BOB, "USDC", 800.0);

    let res = t.try_borrow(BOB, "USDC", 80.0);
    assert_contract_error(res, errors::UTILIZATION_ABOVE_MAX);
}

#[test]
fn test_borrow_at_max_utilization_succeeds() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .without_initial_liquidity()
        .with_market_params("USDC", |p| {
            p.max_utilization = RAY * 85 / 100;
        })
        .build();

    t.supply(ALICE, "USDC", 1_000.0);
    t.supply(BOB, "ETH", 10.0);

    t.borrow(BOB, "USDC", 850.0);

    t.assert_borrow_near(BOB, "USDC", 850.0, 0.01);
    t.assert_healthy(BOB);
}

#[test]
fn test_max_utilization_uses_index_aware_ratio() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .without_initial_liquidity()
        .with_market_params("USDC", |p| {
            p.max_utilization = RAY * 85 / 100;
        })
        .build();

    t.supply(ALICE, "USDC", 1_000.0);
    t.supply(BOB, "ETH", 1_000.0);
    t.borrow(BOB, "USDC", 800.0);

    // Two years of interest lift index-aware utilization past the 85% cap
    // while cash still covers the liquidation buffer.
    t.advance_time(60 * 60 * 24 * 365 * 2);
    t.update_indexes_for(&["USDC"]);

    let result = t.try_borrow(BOB, "USDC", 1.0);
    assert_contract_error(result, errors::UTILIZATION_ABOVE_MAX);
}

#[test]
fn test_withdraw_pushing_above_max_utilization_rejected() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .without_initial_liquidity()
        .with_market_params("USDC", |p| {
            p.max_utilization = RAY * 85 / 100;
        })
        .build();

    t.supply(ALICE, "USDC", 1_000.0);
    t.supply(BOB, "ETH", 10.0);
    t.borrow(BOB, "USDC", 800.0);

    let res = t.try_withdraw(ALICE, "USDC", 200.0);
    assert_contract_error(res, errors::UTILIZATION_ABOVE_MAX);
}

#[test]
fn test_zero_supply_with_outstanding_borrow_rejected() {
    use test_harness::helpers;

    let mut t = LendingTest::new()
        .standard_two_asset()
        .without_initial_liquidity()
        .build();

    t.supply(ALICE, "USDC", 1_000.0);
    t.supply(BOB, "ETH", 10.0);
    t.borrow(BOB, "USDC", 500.0);

    // Share-less cash lets the last supplier's full exit pass the reserve
    // check and reach the supply-for-debt guard.
    let decimals = t.resolve_market("USDC").decimals;
    t.inject_shareless_cash_raw("USDC", helpers::f64_to_i128(10_000.0, decimals));

    use soroban_sdk::Vec as SorobanVec;
    let asset_addr = t.resolve_asset("USDC");
    let alice_addr = t.get_or_create_user(ALICE);
    let account_id = t.resolve_account_id(ALICE);
    let withdrawals: SorobanVec<(HubAssetKey, i128)> =
        soroban_sdk::vec![&t.env, (hub_asset(asset_addr), 0i128)];
    let ctrl = t.ctrl_client();
    let result = match ctrl.try_withdraw(&alice_addr, &account_id, &withdrawals, &None) {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(err)) => Err(err.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(result, errors::POOL_INSOLVENT);
}

/// Every USDC supplier and borrower exits, leaving rounding dust in cash with
/// zero supply shares. Borrowing that dust would mint debt without supply
/// (INV-ACCT-09) and make every later revenue claim on the market revert.
#[test]
fn test_borrow_into_zero_supply_market_rejected() {
    let mut t = LendingTest::new()
        .with_market(MarketPreset {
            initial_liquidity: 0.0,
            ..usdc_preset()
        })
        .with_market(MarketPreset {
            initial_liquidity: 0.0,
            ..wbtc_preset()
        })
        .build();
    let accumulator = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    t.ctrl_client().set_accumulator(&accumulator);

    t.supply(ALICE, "USDC", 1_000.0);
    t.supply(BOB, "WBTC", 1.0);
    t.borrow(BOB, "USDC", 500.0);
    t.advance_time(days(365));
    t.repay(BOB, "USDC", 600.0);
    t.withdraw_all(ALICE, "USDC");
    t.claim_revenue("USDC");

    let usdc = hub_asset(t.resolve_asset("USDC"));
    let emptied = t.pool_client("USDC").get_sync_data(&usdc).state;
    assert_eq!(
        (emptied.supplied, emptied.borrowed, emptied.revenue),
        (0, 0, 0)
    );
    assert!(emptied.cash > 0, "rounding leaves dust in cash");

    t.supply(CAROL, "WBTC", 0.1);
    let carol = t.get_or_create_user(CAROL);
    let carol_account = t.resolve_account_id(CAROL);
    let borrows: soroban_sdk::Vec<(HubAssetKey, i128)> =
        soroban_sdk::vec![&t.env, (usdc.clone(), emptied.cash)];
    let result =
        map_try_ok_unit(
            t.ctrl_client()
                .try_borrow(&carol, &carol_account, &borrows, &None),
        );

    assert_contract_error(result, errors::POOL_INSOLVENT);
    assert_eq!(t.pool_client("USDC").get_sync_data(&usdc).state.borrowed, 0);
    assert_eq!(t.try_claim_revenue("USDC"), Ok(0));
}

#[test]
fn test_update_params_rejects_max_below_optimal() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let model = controller::types::InterestRateModel {
        max_borrow_rate: RAY,
        base_borrow_rate: RAY / 100,
        slope1: RAY * 4 / 100,
        slope2: RAY * 10 / 100,
        slope3: RAY * 80 / 100,
        mid_utilization: RAY * 50 / 100,
        optimal_utilization: RAY * 80 / 100,

        max_utilization: RAY * 70 / 100,
        reserve_factor: 1000,
        is_flashloanable: false,
        flashloan_fee: 0,
    };
    let asset = t.resolve_asset("USDC");
    let result = t
        .ctrl_client()
        .try_upgrade_liquidity_pool_params(&hub_asset(asset), &model);
    let mapped = map_try_ok_unit(result);
    assert_contract_error(mapped, errors::INVALID_UTIL_RANGE);
}

#[test]
fn test_update_params_rejects_max_above_one() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let model = controller::types::InterestRateModel {
        max_borrow_rate: RAY,
        base_borrow_rate: RAY / 100,
        slope1: RAY * 4 / 100,
        slope2: RAY * 10 / 100,
        slope3: RAY * 80 / 100,
        mid_utilization: RAY * 50 / 100,
        optimal_utilization: RAY * 80 / 100,
        max_utilization: RAY + 1,
        reserve_factor: 1000,
        is_flashloanable: false,
        flashloan_fee: 0,
    };
    let asset = t.resolve_asset("USDC");
    let result = t
        .ctrl_client()
        .try_upgrade_liquidity_pool_params(&hub_asset(asset), &model);
    let mapped = map_try_ok_unit(result);
    assert_contract_error(mapped, errors::INVALID_UTIL_RANGE);
}

/// GH-03. The pool admits utilization (RAY, rounded half-up) `<= max_utilization`.
/// Pins the last admissible withdrawal and the first rejected one, one raw unit apart.
#[test]
fn withdraw_that_lands_exactly_on_max_utilization_passes_and_one_unit_more_fails() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .without_initial_liquidity()
        .with_market_params("USDC", |p| p.max_utilization = RAY * 80 / 100)
        .with_min_borrow_collateral_disabled()
        .build();
    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(BOB, "ETH", 1_000.0);
    t.borrow(BOB, "USDC", 40_000.0);
    // Supplied 100_000, borrowed 40_000. Utilization hits 80 percent when
    // supplied falls to 50_000: ALICE may withdraw exactly 50_000.
    let unit = 10_000_000i128;
    let exact = 50_000 * unit;
    assert_contract_error(
        t.try_withdraw_raw(ALICE, "USDC", exact + 1),
        errors::UTILIZATION_ABOVE_MAX,
    );
    t.try_withdraw_raw(ALICE, "USDC", exact)
        .expect("landing exactly on the ceiling is admissible");
    assert_contract_error(
        t.try_withdraw_raw(ALICE, "USDC", 1),
        errors::UTILIZATION_ABOVE_MAX,
    );
}
