use test_harness::{
    days, eth_preset, usd, usdc_preset, LendingTest, ALICE, BOB, CAROL, DAVE, EVE, LIQUIDATOR,
};

#[test]
fn test_multi_user_lending_cycle() {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();

    t.supply(ALICE, "USDC", 50_000.0);
    t.supply(BOB, "ETH", 10.0);
    t.supply(CAROL, "WBTC", 0.5);
    t.supply(DAVE, "USDC", 100_000.0);
    t.supply(EVE, "ETH", 5.0);

    t.borrow(ALICE, "ETH", 5.0);
    t.borrow(BOB, "USDC", 10_000.0);
    t.borrow(CAROL, "USDC", 15_000.0);

    t.assert_healthy(ALICE);
    t.assert_healthy(BOB);
    t.assert_healthy(CAROL);

    let revenue_eth_0 = t.snapshot_revenue("ETH");
    let revenue_usdc_0 = t.snapshot_revenue("USDC");

    t.advance_and_sync(days(7));

    t.supply("user6", "USDC", 20_000.0);
    t.supply("user7", "ETH", 3.0);
    t.supply("user8", "WBTC", 0.2);
    t.supply("user9", "USDC", 30_000.0);
    t.supply("user10", "ETH", 2.0);

    t.borrow(DAVE, "ETH", 10.0);
    t.borrow(EVE, "WBTC", 0.05);

    t.repay(ALICE, "ETH", 2.0);

    t.assert_healthy(ALICE);
    t.assert_healthy(DAVE);
    t.assert_healthy(EVE);

    t.advance_and_sync(days(7));

    t.set_price("WBTC", usd(25_000));

    t.assert_liquidatable(CAROL);

    t.assert_healthy(ALICE);
    t.assert_healthy(BOB);
    t.assert_healthy(DAVE);

    t.liquidate("user8", CAROL, "USDC", 5_000.0);

    t.set_price("WBTC", usd(60_000));
    t.advance_and_sync(days(7));

    t.borrow("user6", "ETH", 2.0);
    t.repay(BOB, "USDC", 5_000.0);

    t.assert_healthy("user6");
    t.assert_healthy(BOB);

    t.advance_and_sync(days(7));

    t.assert_healthy(ALICE);
    t.assert_healthy(BOB);
    t.assert_healthy(DAVE);
    t.assert_healthy(EVE);
    t.assert_healthy("user6");

    let revenue_eth_final = t.snapshot_revenue("ETH");
    let revenue_usdc_final = t.snapshot_revenue("USDC");
    assert!(
        revenue_eth_final > revenue_eth_0,
        "ETH revenue should have increased: before={}, after={}",
        revenue_eth_0,
        revenue_eth_final
    );
    assert!(
        revenue_usdc_final > revenue_usdc_0,
        "USDC revenue should have increased: before={}, after={}",
        revenue_usdc_0,
        revenue_usdc_final
    );
}

#[test]
fn test_full_exit_solvency() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(BOB, "USDC", 50_000.0);
    t.supply(CAROL, "ETH", 20.0);
    t.supply(DAVE, "USDC", 80_000.0);
    t.supply(EVE, "ETH", 10.0);

    t.borrow(ALICE, "ETH", 10.0);
    t.borrow(BOB, "ETH", 5.0);
    t.borrow(CAROL, "USDC", 20_000.0);

    t.advance_and_sync(days(90));

    let revenue_eth_before = t.snapshot_revenue("ETH");
    let revenue_usdc_before = t.snapshot_revenue("USDC");

    let alice_debt = t.borrow_balance(ALICE, "ETH");
    assert!(
        alice_debt > 10.0,
        "Alice's debt should have grown from 10 ETH, got {}",
        alice_debt
    );

    t.repay(ALICE, "ETH", 15.0);
    t.repay(BOB, "ETH", 10.0);
    t.repay(CAROL, "USDC", 30_000.0);

    let alice_debt_after = t.borrow_balance(ALICE, "ETH");
    assert!(
        alice_debt_after < 0.001,
        "Alice's debt should be ~0 after repay, got {}",
        alice_debt_after
    );

    // Dave is a pure supplier, so his exit is fully determined: the wallet must
    // move by exactly his credited balance. Without this the test could not see
    // a bug that under-paid every withdrawer and stranded the surplus.
    let dave_credited = t.supply_balance_raw(DAVE, "USDC");
    let dave_wallet_before = t.token_balance_raw(DAVE, "USDC");

    t.withdraw_all(ALICE, "USDC");
    t.withdraw_all(BOB, "USDC");
    t.withdraw_all(CAROL, "ETH");
    t.withdraw_all(DAVE, "USDC");
    t.withdraw_all(EVE, "ETH");

    // Exact to the stroop, and directional: withdraw floors in the protocol's
    // favour (ADR-0003), so the payout is the credited balance or one stroop
    // under it -- never over, and never 5% under.
    let dave_paid = t.token_balance_raw(DAVE, "USDC") - dave_wallet_before;
    assert!(
        dave_paid == dave_credited || dave_paid == dave_credited - 1,
        "dave's payout must equal his credited supply within the protocol-favouring \
         floor: paid={dave_paid}, credited={dave_credited}"
    );

    // `pool_reserves` is `state.cash`, which the builder pre-loads with
    // `initial_liquidity` (src/multi_hub.rs:80-95). `>= 0.0` therefore had a
    // million-unit margin. Everyone has exited, so the only cash that may
    // remain is that donation plus the unclaimed protocol revenue.
    let donated_usdc = usdc_preset().initial_liquidity;
    let donated_eth = eth_preset().initial_liquidity;
    let reserves_usdc = t.pool_reserves("USDC");
    let reserves_eth = t.pool_reserves("ETH");
    assert!(
        reserves_usdc >= donated_usdc,
        "USDC pool leaked below its seeded liquidity: reserves = {}, seeded = {}",
        reserves_usdc,
        donated_usdc
    );
    assert!(
        reserves_eth >= donated_eth,
        "ETH pool leaked below its seeded liquidity: reserves = {}, seeded = {}",
        reserves_eth,
        donated_eth
    );

    assert!(
        revenue_eth_before > 0,
        "ETH revenue should be positive: {}",
        revenue_eth_before
    );
    assert!(
        revenue_usdc_before > 0,
        "USDC revenue should be positive: {}",
        revenue_usdc_before
    );
}

#[test]
fn test_cascading_liquidations_stability() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "ETH", 10.0);
    t.borrow(ALICE, "USDC", 14_500.0);

    t.supply(BOB, "ETH", 10.0);
    t.borrow(BOB, "USDC", 12_300.0);

    t.supply(CAROL, "ETH", 10.0);
    t.borrow(CAROL, "USDC", 10_000.0);

    t.supply(DAVE, "ETH", 10.0);
    t.borrow(DAVE, "USDC", 8_000.0);

    t.supply(EVE, "ETH", 10.0);
    t.borrow(EVE, "USDC", 5_300.0);

    t.assert_healthy(ALICE);
    t.assert_healthy(BOB);
    t.assert_healthy(CAROL);
    t.assert_healthy(DAVE);
    t.assert_healthy(EVE);

    t.set_price("ETH", usd(1600));

    t.assert_liquidatable(ALICE);
    t.assert_healthy(BOB);
    t.assert_healthy(CAROL);
    t.assert_healthy(DAVE);
    t.assert_healthy(EVE);

    let alice_debt_before = t.total_debt(ALICE);
    t.liquidate(LIQUIDATOR, ALICE, "USDC", 5_000.0);
    let alice_debt_after = t.total_debt(ALICE);
    assert!(
        alice_debt_after < alice_debt_before,
        "Liquidation should reduce Alice's debt: before={}, after={}",
        alice_debt_before,
        alice_debt_after
    );

    t.set_price("ETH", usd(1300));

    t.assert_liquidatable(BOB);
    t.assert_healthy(CAROL);
    t.assert_healthy(DAVE);
    t.assert_healthy(EVE);

    let bob_debt_before = t.total_debt(BOB);
    t.liquidate(LIQUIDATOR, BOB, "USDC", 4_000.0);
    let bob_debt_after = t.total_debt(BOB);
    assert!(
        bob_debt_after < bob_debt_before,
        "Liquidation should reduce Bob's debt: before={}, after={}",
        bob_debt_before,
        bob_debt_after
    );

    t.set_price("ETH", usd(1000));

    t.assert_liquidatable(CAROL);
    t.assert_healthy(EVE);

    let carol_partial = t.try_liquidate(LIQUIDATOR, CAROL, "USDC", 3_000.0);
    assert!(
        carol_partial.is_err(),
        "solvent-toxic partial must be rejected"
    );
    t.liquidate(LIQUIDATOR, CAROL, "USDC", 10_100.0);

    let eve_result = t.try_liquidate(LIQUIDATOR, EVE, "USDC", 1_000.0);
    assert!(
        eve_result.is_err(),
        "Eve should not be liquidatable (HF > 1.0)"
    );
}

#[test]
fn test_interest_accrual_consistency() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(BOB, "ETH", 100.0);
    t.borrow(ALICE, "ETH", 25.0);

    let mut prev_debt = t.borrow_balance(ALICE, "ETH");
    let mut prev_supply = t.supply_balance(BOB, "ETH");

    let intervals = [days(1), days(7), days(30), days(90), days(365)];

    for (i, &interval) in intervals.iter().enumerate() {
        t.advance_and_sync(interval);

        let current_debt = t.borrow_balance(ALICE, "ETH");
        let current_supply = t.supply_balance(BOB, "ETH");

        assert!(
            current_debt > prev_debt,
            "Debt should increase at checkpoint {}: prev={}, current={}",
            i,
            prev_debt,
            current_debt
        );

        assert!(
            current_supply > prev_supply,
            "Supply should increase at checkpoint {}: prev={}, current={}",
            i,
            prev_supply,
            current_supply
        );

        prev_debt = current_debt;
        prev_supply = current_supply;
    }

    let revenue_eth = t.snapshot_revenue("ETH");
    assert!(
        revenue_eth > 0,
        "ETH revenue should be positive after interest accrual: {}",
        revenue_eth
    );

    let total_supply_interest = t.supply_balance(BOB, "ETH") - 100.0;
    let _total_borrow_interest = t.borrow_balance(ALICE, "ETH") - 25.0;

    assert!(
        total_supply_interest > 0.0,
        "Supply interest should be positive: {}",
        total_supply_interest
    );
}

#[test]
fn test_position_limit_exactly_at_cap() {
    let mut t = LendingTest::new()
        .three_asset_usdc_eth_wbtc()
        .with_position_limits(3, 3)
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(ALICE, "ETH", 10.0);
    t.supply(ALICE, "WBTC", 1.0);

    t.assert_supply_count(ALICE, 3);

    t.borrow(ALICE, "USDC", 1_000.0);
    t.borrow(ALICE, "ETH", 0.5);
    t.borrow(ALICE, "WBTC", 0.01);

    t.assert_borrow_count(ALICE, 3);
    t.assert_healthy(ALICE);

    t.repay(ALICE, "WBTC", 1.0);

    t.assert_borrow_count(ALICE, 2);

    t.borrow(ALICE, "WBTC", 0.005);
    t.assert_borrow_count(ALICE, 3);
    t.assert_healthy(ALICE);
}

#[test]
fn test_keeper_index_freshness_matters() {
    let mut t_a = LendingTest::new().standard_two_asset().build();

    t_a.supply(BOB, "ETH", 50.0);
    t_a.supply(ALICE, "USDC", 100_000.0);
    t_a.borrow(ALICE, "ETH", 10.0);

    t_a.advance_and_sync(days(30));

    let debt_a = t_a.borrow_balance(ALICE, "ETH");
    let revenue_a = t_a.snapshot_revenue("ETH");

    let mut t_b = LendingTest::new().standard_two_asset().build();

    t_b.supply(BOB, "ETH", 50.0);
    t_b.supply(ALICE, "USDC", 100_000.0);
    t_b.borrow(ALICE, "ETH", 10.0);

    for _ in 0..30 {
        t_b.advance_and_sync(days(1));
    }

    let debt_b = t_b.borrow_balance(ALICE, "ETH");
    let revenue_b = t_b.snapshot_revenue("ETH");

    assert!(
        debt_a > 10.0,
        "Scenario A debt should be > 10 ETH: {}",
        debt_a
    );
    assert!(
        debt_b > 10.0,
        "Scenario B debt should be > 10 ETH: {}",
        debt_b
    );

    assert!(
        revenue_a > 0,
        "Scenario A revenue should be positive: {}",
        revenue_a
    );
    assert!(
        revenue_b > 0,
        "Scenario B revenue should be positive: {}",
        revenue_b
    );

    assert!(
        debt_b > debt_a,
        "Daily syncing must compound to strictly more debt than one sync: A={}, B={}",
        debt_a,
        debt_b
    );
    assert!(
        revenue_b > revenue_a,
        "Daily syncing must compound to strictly more revenue than one sync: A={}, B={}",
        revenue_a,
        revenue_b
    );
}
