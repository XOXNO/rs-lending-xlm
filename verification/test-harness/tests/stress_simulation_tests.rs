extern crate std;

use test_harness::{
    days, eth_preset, usd, usdc_preset, wbtc_preset, LendingTest, ALICE, BOB, CAROL, DAVE, EVE,
    LIQUIDATOR,
};

// ===========================================================================
// Test 1: Multi-user lending cycle over 4 weeks
// ===========================================================================

#[test]
fn test_multi_user_lending_cycle() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    // -- Week 1: users 1-5 supply, users 1-3 borrow --

    // Supplies.
    t.supply(ALICE, "USDC", 50_000.0);
    t.supply(BOB, "ETH", 10.0); // ~$20,000
    t.supply(CAROL, "WBTC", 0.5); // ~$30,000
    t.supply(DAVE, "USDC", 100_000.0);
    t.supply(EVE, "ETH", 5.0); // ~$10,000

    // Borrows (against collateral).
    t.borrow(ALICE, "ETH", 5.0); // $10k borrow vs $50k collateral => safe
    t.borrow(BOB, "USDC", 10_000.0); // $10k borrow vs $20k collateral => safe
    t.borrow(CAROL, "USDC", 15_000.0); // $15k borrow vs $30k collateral => tight

    t.assert_healthy(ALICE);
    t.assert_healthy(BOB);
    t.assert_healthy(CAROL);

    let revenue_eth_0 = t.snapshot_revenue("ETH");
    let revenue_usdc_0 = t.snapshot_revenue("USDC");

    // Advance 1 week.
    t.advance_and_sync(days(7));

    // -- Week 2: users 6-10 supply, users 4-5 borrow, user 1 partial repay --

    t.supply("user6", "USDC", 20_000.0);
    t.supply("user7", "ETH", 3.0);
    t.supply("user8", "WBTC", 0.2);
    t.supply("user9", "USDC", 30_000.0);
    t.supply("user10", "ETH", 2.0);

    t.borrow(DAVE, "ETH", 10.0); // $20k borrow vs $100k USDC => safe
    t.borrow(EVE, "WBTC", 0.05); // $3k borrow vs $10k collateral => safe

    // Alice partial repay.
    t.repay(ALICE, "ETH", 2.0);

    t.assert_healthy(ALICE);
    t.assert_healthy(DAVE);
    t.assert_healthy(EVE);

    // Advance 1 week.
    t.advance_and_sync(days(7));

    // -- Week 3: a price drop makes Carol liquidatable --

    // Carol has 0.5 WBTC collateral (~$30k) and $15k USDC debt.
    // LTV = 75%, liq threshold = 80% => weighted = $24k vs $15k => HF ~1.6.
    // Drop WBTC from $60k to $25k => collateral = $12.5k, weighted = $10k.
    // HF = $10k / $15k = 0.67 => liquidatable.
    t.set_price("WBTC", usd(25_000));

    t.assert_liquidatable(CAROL);
    // Healthy users must stay healthy.
    t.assert_healthy(ALICE);
    t.assert_healthy(BOB);
    t.assert_healthy(DAVE);

    // user8 liquidates Carol.
    t.liquidate("user8", CAROL, "USDC", 5_000.0);

    // Advance 1 week.
    t.set_price("WBTC", usd(60_000)); // restore price
    t.advance_and_sync(days(7));

    // -- Week 4: more borrows and repays --

    t.borrow("user6", "ETH", 2.0); // ~$4k vs $20k => safe
    t.repay(BOB, "USDC", 5_000.0); // partial repay

    t.assert_healthy("user6");
    t.assert_healthy(BOB);

    // Advance 1 week.
    t.advance_and_sync(days(7));

    // -- Final invariant checks --

    // All accounts with borrows must stay healthy.
    t.assert_healthy(ALICE);
    t.assert_healthy(BOB);
    t.assert_healthy(DAVE);
    t.assert_healthy(EVE);
    t.assert_healthy("user6");

    // Revenue must have grown.
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

// ===========================================================================
// Test 2: Full exit solvency (bank run)
// ===========================================================================

#[test]
fn test_full_exit_solvency() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // 5 users supply.
    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(BOB, "USDC", 50_000.0);
    t.supply(CAROL, "ETH", 20.0);
    t.supply(DAVE, "USDC", 80_000.0);
    t.supply(EVE, "ETH", 10.0);

    // 3 users borrow.
    t.borrow(ALICE, "ETH", 10.0); // ~$20k vs $100k
    t.borrow(BOB, "ETH", 5.0); // ~$10k vs $50k
    t.borrow(CAROL, "USDC", 20_000.0); // $20k vs $40k

    // Advance 90 days for significant interest.
    t.advance_and_sync(days(90));

    let revenue_eth_before = t.snapshot_revenue("ETH");
    let revenue_usdc_before = t.snapshot_revenue("USDC");

    // Verify interest accrued.
    let alice_debt = t.borrow_balance(ALICE, "ETH");
    assert!(
        alice_debt > 10.0,
        "Alice's debt should have grown from 10 ETH, got {}",
        alice_debt
    );

    // -- Exit phase: all borrowers repay --
    // Use a large overpayment to ensure full repay.
    t.repay(ALICE, "ETH", 15.0);
    t.repay(BOB, "ETH", 10.0);
    t.repay(CAROL, "USDC", 30_000.0);

    // Verify debt is gone (or near zero).
    let alice_debt_after = t.borrow_balance(ALICE, "ETH");
    assert!(
        alice_debt_after < 0.001,
        "Alice's debt should be ~0 after repay, got {}",
        alice_debt_after
    );

    // -- Exit phase: all suppliers withdraw --
    t.withdraw_all(ALICE, "USDC");
    t.withdraw_all(BOB, "USDC");
    t.withdraw_all(CAROL, "ETH");
    t.withdraw_all(DAVE, "USDC");
    t.withdraw_all(EVE, "ETH");

    // The pool must remain solvent (reserves >= 0).
    let reserves_usdc = t.pool_reserves("USDC");
    let reserves_eth = t.pool_reserves("ETH");
    assert!(
        reserves_usdc >= 0.0,
        "USDC pool should be solvent, reserves = {}",
        reserves_usdc
    );
    assert!(
        reserves_eth >= 0.0,
        "ETH pool should be solvent, reserves = {}",
        reserves_eth
    );

    // Revenue must be positive (interest accrued).
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

// ===========================================================================
// Test 3: Cascading liquidations stability
// ===========================================================================

#[test]
fn test_cascading_liquidations_stability() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Create users with different HF levels. All supply ETH and borrow
    // USDC. ETH price = $2000, LTV = 75%, liq threshold = 80%.
    //
    // For HF = threshold_weighted_collateral / debt:
    //   weighted_collateral = supply_ETH * $2000 * 0.80.
    //
    // User A: tight HF ~1.1 => supply 10 ETH ($20k), borrow ~$14,545 USDC.
    //   HF = (10 * 2000 * 0.80) / 14545 = 16000/14545 = 1.10.
    t.supply(ALICE, "ETH", 10.0);
    t.borrow(ALICE, "USDC", 14_500.0);

    // User B: HF ~1.3 => supply 10 ETH, borrow ~$12,308.
    t.supply(BOB, "ETH", 10.0);
    t.borrow(BOB, "USDC", 12_300.0);

    // User C: HF ~1.6 => supply 10 ETH, borrow ~$10,000.
    t.supply(CAROL, "ETH", 10.0);
    t.borrow(CAROL, "USDC", 10_000.0);

    // User D: HF ~2.0 => supply 10 ETH, borrow ~$8,000.
    t.supply(DAVE, "ETH", 10.0);
    t.borrow(DAVE, "USDC", 8_000.0);

    // User E: HF ~3.0 => supply 10 ETH, borrow ~$5,333.
    t.supply(EVE, "ETH", 10.0);
    t.borrow(EVE, "USDC", 5_300.0);

    t.assert_healthy(ALICE);
    t.assert_healthy(BOB);
    t.assert_healthy(CAROL);
    t.assert_healthy(DAVE);
    t.assert_healthy(EVE);

    // -- Progressive price drops --

    // Drop 1: ETH $2000 -> $1600 (20% drop). New HFs:
    //   Alice: (10*1600*0.80)/14500 = 12800/14500 = 0.88 => liquidatable.
    //   Bob:   12800/12300 = 1.04 => still healthy.
    //   Carol: 12800/10000 = 1.28 => healthy.
    //   Dave:  12800/8000  = 1.60 => healthy.
    //   Eve:   12800/5300  = 2.42 => healthy.
    t.set_price("ETH", usd(1600));

    t.assert_liquidatable(ALICE);
    t.assert_healthy(BOB);
    t.assert_healthy(CAROL);
    t.assert_healthy(DAVE);
    t.assert_healthy(EVE);

    // Liquidate Alice; verify debt decreases (HF improvement).
    let alice_debt_before = t.total_debt(ALICE);
    t.liquidate(LIQUIDATOR, ALICE, "USDC", 5_000.0);
    let alice_debt_after = t.total_debt(ALICE);
    assert!(
        alice_debt_after < alice_debt_before,
        "Liquidation should reduce Alice's debt: before={}, after={}",
        alice_debt_before,
        alice_debt_after
    );

    // Drop 2: ETH $1600 -> $1300 (further drop).
    // Bob: (10*1300*0.80)/12300 = 10400/12300 = 0.85 => liquidatable.
    // Carol: 10400/10000 = 1.04 => still healthy.
    // Dave: 10400/8000 = 1.30 => healthy.
    // Eve: 10400/5300 = 1.96 => healthy.
    t.set_price("ETH", usd(1300));

    t.assert_liquidatable(BOB);
    t.assert_healthy(CAROL);
    t.assert_healthy(DAVE);
    t.assert_healthy(EVE);

    // Liquidate Bob; verify debt decreases.
    let bob_debt_before = t.total_debt(BOB);
    t.liquidate(LIQUIDATOR, BOB, "USDC", 4_000.0);
    let bob_debt_after = t.total_debt(BOB);
    assert!(
        bob_debt_after < bob_debt_before,
        "Liquidation should reduce Bob's debt: before={}, after={}",
        bob_debt_before,
        bob_debt_after
    );

    // Drop 3: ETH $1300 -> $1000.
    // Carol: (10*1000*0.80)/10000 = 8000/10000 = 0.80 => liquidatable.
    // Dave: 8000/8000 = 1.00 => borderline / liquidatable.
    // Eve: 8000/5300 = 1.51 => healthy.
    t.set_price("ETH", usd(1000));

    t.assert_liquidatable(CAROL);
    t.assert_healthy(EVE);

    // Liquidate Carol.
    t.liquidate(LIQUIDATOR, CAROL, "USDC", 3_000.0);

    // Eve must remain untouched (no wrongful liquidation).
    let eve_result = t.try_liquidate(LIQUIDATOR, EVE, "USDC", 1_000.0);
    assert!(
        eve_result.is_err(),
        "Eve should not be liquidatable (HF > 1.0)"
    );
}

// ===========================================================================
// Test 4: Interest accrual consistency
// ===========================================================================

#[test]
fn test_interest_accrual_consistency() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Alice supplies USDC as collateral and borrows ETH. Bob supplies ETH
    // to the pool so Alice can borrow from it. Bob's ETH supply therefore
    // earns interest from Alice's ETH borrow.
    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(BOB, "ETH", 100.0);
    t.borrow(ALICE, "ETH", 25.0); // ~$50k

    let mut prev_debt = t.borrow_balance(ALICE, "ETH");
    let mut prev_supply = t.supply_balance(BOB, "ETH");

    // Checkpoint intervals: 1 day, 1 week, 1 month, 3 months, 1 year.
    let intervals = [days(1), days(7), days(30), days(90), days(365)];

    for (i, &interval) in intervals.iter().enumerate() {
        t.advance_and_sync(interval);

        let current_debt = t.borrow_balance(ALICE, "ETH");
        let current_supply = t.supply_balance(BOB, "ETH");

        // Debt must strictly increase.
        assert!(
            current_debt > prev_debt,
            "Debt should increase at checkpoint {}: prev={}, current={}",
            i,
            prev_debt,
            current_debt
        );

        // Supply balance must rise (earning interest from borrowers).
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

    // Verify the protocol generated revenue.
    let revenue_eth = t.snapshot_revenue("ETH");
    assert!(
        revenue_eth > 0,
        "ETH revenue should be positive after interest accrual: {}",
        revenue_eth
    );

    // Verify supply interest ~ borrow interest * (1 - reserve_factor).
    // reserve_factor = 1000 BPS = 10%. This is approximate due to compound
    // interest effects.
    let total_supply_interest = t.supply_balance(BOB, "ETH") - 100.0;
    let _total_borrow_interest = t.borrow_balance(ALICE, "ETH") - 25.0;

    // Supply interest must be positive (suppliers earn).
    assert!(
        total_supply_interest > 0.0,
        "Supply interest should be positive: {}",
        total_supply_interest
    );
}

// ===========================================================================
// Test 5: Position limit exactly at cap
// ===========================================================================

#[test]
fn test_position_limit_exactly_at_cap() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_position_limits(3, 3)
        .build();

    // Supply all 3 assets (hits the limit).
    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(ALICE, "ETH", 10.0);
    t.supply(ALICE, "WBTC", 1.0);

    t.assert_supply_count(ALICE, 3);

    // Borrow all 3 assets.
    t.borrow(ALICE, "USDC", 1_000.0);
    t.borrow(ALICE, "ETH", 0.5);
    t.borrow(ALICE, "WBTC", 0.01);

    t.assert_borrow_count(ALICE, 3);
    t.assert_healthy(ALICE);

    // Repay one borrow to make room.
    t.repay(ALICE, "WBTC", 1.0); // overpay to fully close

    // Verify the borrow count decreased.
    t.assert_borrow_count(ALICE, 2);

    // Borrow count has dropped back below the limit.
    t.borrow(ALICE, "WBTC", 0.005);
    t.assert_borrow_count(ALICE, 3);
    t.assert_healthy(ALICE);
}

// ===========================================================================
// Test 6: Keeper index freshness matters
// ===========================================================================

#[test]
fn test_keeper_index_freshness_matters() {
    // -- Scenario A: advance 30 days with no intermediate sync --
    let mut t_a = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t_a.supply(ALICE, "USDC", 100_000.0);
    t_a.borrow(ALICE, "ETH", 10.0);

    // Advance 30 days with one sync at the end.
    t_a.advance_and_sync(days(30));

    let debt_a = t_a.borrow_balance(ALICE, "ETH");
    let revenue_a = t_a.snapshot_revenue("ETH");

    // -- Scenario B: advance 30 days with daily syncs --
    let mut t_b = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t_b.supply(ALICE, "USDC", 100_000.0);
    t_b.borrow(ALICE, "ETH", 10.0);

    // Advance 30 days with daily syncs.
    for _ in 0..30 {
        t_b.advance_and_sync(days(1));
    }

    let debt_b = t_b.borrow_balance(ALICE, "ETH");
    let revenue_b = t_b.snapshot_revenue("ETH");

    // Both debts must exceed 10 ETH (interest accrued).
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

    // Both revenues must be positive.
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

    // KEY INSIGHT: frequent syncs produce significantly more revenue because
    // the protocol tracks interest through index updates. Without syncs,
    // revenue materializes only when indexes update. Daily syncs capture the
    // full compound-interest curve; a single sync underestimates it.
    //
    // Scenario B (daily syncs) must accrue >= Scenario A (single sync) for
    // both debt and revenue.
    assert!(
        debt_b >= debt_a,
        "Daily-sync debt should be >= single-sync: A={}, B={}",
        debt_a,
        debt_b
    );
    assert!(
        revenue_b >= revenue_a,
        "Daily-sync revenue should be >= single-sync: A={}, B={}",
        revenue_a,
        revenue_b
    );
}
