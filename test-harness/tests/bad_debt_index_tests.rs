extern crate std;

use test_harness::{
    days, eth_preset, usd, usd_cents, usdc_preset, LendingTest, ALICE, BOB, CAROL, DAVE, LIQUIDATOR,
};

// ===========================================================================
// Bad debt supply index tests -- the only case where supply_index decreases.
//
// When debt exceeds collateral and collateral < $5:
//   1. Seize all remaining collateral (dust -> protocol revenue).
//   2. Socialize remaining debt via pool.seize_position(borrow_pos).
//   3. Pool calls apply_bad_debt_to_supply_index(debt_amount).
//   4. Reduce supply index: new = old * (total - bad_debt) / total.
//   5. Every supplier's balance shrinks proportionally.
//
// This is the protocol's loss-distribution mechanism: suppliers absorb the
// loss that liquidation could not recover.
// ===========================================================================

fn get_indexes(t: &LendingTest, asset: &str) -> (i128, i128) {
    let asset_addr = t.resolve_asset(asset);
    let ctrl = t.ctrl_client();
    let assets = soroban_sdk::Vec::from_array(&t.env, [asset_addr]);
    let idx = ctrl
        .get_all_market_indexes_detailed(&assets)
        .get(0)
        .unwrap();
    (idx.supply_index_ray, idx.borrow_index_ray)
}

// ---------------------------------------------------------------------------
// 1. Supply index decreases after bad debt socialization
// ---------------------------------------------------------------------------

#[test]
fn test_bad_debt_decreases_supply_index() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Bob supplies ETH and will absorb the bad-debt loss.
    t.supply(BOB, "ETH", 100.0);

    // Alice supplies small USDC collateral and borrows ETH.
    t.supply(ALICE, "USDC", 10.0); // $10 collateral.
    t.borrow(ALICE, "ETH", 0.003); // $6 debt at $2000/ETH.

    let (si_before, _) = get_indexes(&t, "ETH");

    // Crash USDC to $0.10: collateral = $1, debt = $6.
    // HF = ($1 * 0.80) / $6 = 0.13.
    t.set_price("USDC", usd_cents(10));
    t.assert_liquidatable(ALICE);

    // Liquidate; bad-debt cleanup fires because collateral < $5.
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.001);

    let (si_after, _) = get_indexes(&t, "ETH");

    // Bad-debt socialization must drop the supply index.
    assert!(
        si_after < si_before,
        "supply index should DECREASE after bad debt: before={}, after={}",
        si_before,
        si_after
    );

    // The drop must be proportional to bad_debt / total_supplied.
    // Bad debt ~ 0.002 ETH ($4 of remaining debt after partial liquidation).
    // Total supplied ~ 100 ETH (Bob's supply).
    // Expected reduction ~ 0.002/100 = 0.002% of the index.
    let decrease_ratio = si_after as f64 / si_before as f64;
    assert!(
        decrease_ratio > 0.99 && decrease_ratio < 1.0,
        "decrease should be small relative to total supply: ratio={:.6}",
        decrease_ratio
    );
}

// ---------------------------------------------------------------------------
// 2. All suppliers lose proportionally from bad debt
// ---------------------------------------------------------------------------

#[test]
fn test_bad_debt_loss_distributed_proportionally() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Bob holds 75% and Carol 25% of the ETH supply.
    t.supply(BOB, "ETH", 75.0);
    t.supply(CAROL, "ETH", 25.0);

    // Alice opens a bad-debt position.
    t.supply(ALICE, "USDC", 10.0);
    t.borrow(ALICE, "ETH", 0.003);

    let bob_before = t.supply_balance(BOB, "ETH");
    let carol_before = t.supply_balance(CAROL, "ETH");

    // Crash and liquidate; bad-debt cleanup fires.
    t.set_price("USDC", usd_cents(10));
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.001);

    let bob_after = t.supply_balance(BOB, "ETH");
    let carol_after = t.supply_balance(CAROL, "ETH");

    let bob_loss = bob_before - bob_after;
    let carol_loss = carol_before - carol_after;

    // Both must lose value.
    assert!(
        bob_loss > 0.0,
        "Bob should lose from bad debt: {:.6}",
        bob_loss
    );
    assert!(
        carol_loss > 0.0,
        "Carol should lose from bad debt: {:.6}",
        carol_loss
    );

    // Bob's loss must equal 3x Carol's (75/25 = 3:1).
    if carol_loss > 0.0001 {
        let ratio = bob_loss / carol_loss;
        assert!(
            (ratio - 3.0).abs() < 0.3,
            "loss should be proportional (3:1): ratio={:.4}, bob_loss={:.6}, carol_loss={:.6}",
            ratio,
            bob_loss,
            carol_loss
        );
    }
}

// ---------------------------------------------------------------------------
// 3. Supply index never goes below 1 (floor)
// ---------------------------------------------------------------------------

#[test]
fn test_bad_debt_index_floored_at_one() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Very small supply with large relative bad debt.
    t.supply(BOB, "ETH", 0.01); // Tiny ETH supply ($20).

    // Alice borrows nearly all of it.
    t.supply(ALICE, "USDC", 100.0);
    t.borrow(ALICE, "ETH", 0.005); // $10 debt.

    // Crash USDC fully.
    t.set_price("USDC", usd_cents(1)); // $0.01: collateral = $1.

    // Liquidate; bad debt is large relative to supply.
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.001);

    let (si_after, _) = get_indexes(&t, "ETH");

    // Supply index must remain >= 1 (floored).
    assert!(
        si_after >= 1,
        "supply index should be floored at 1, got {}",
        si_after
    );
}

// ---------------------------------------------------------------------------
// 4. Supply index recovers after bad debt (grows again with new interest)
// ---------------------------------------------------------------------------

#[test]
fn test_supply_index_recovers_after_bad_debt() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10.0);
    t.borrow(ALICE, "ETH", 0.003);

    // Crash and create bad debt.
    t.set_price("USDC", usd_cents(10));
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.001);

    let (si_after_bad_debt, _) = get_indexes(&t, "ETH");

    // Restore the price.
    t.set_price("USDC", usd(1));

    // A new borrower drives utilization, which accrues interest.
    t.supply(DAVE, "USDC", 500_000.0);
    t.borrow(DAVE, "ETH", 30.0);

    // Advance time so interest accrues.
    t.advance_and_sync(days(365));

    let (si_recovered, _) = get_indexes(&t, "ETH");

    // The supply index must grow past the post-bad-debt level.
    assert!(
        si_recovered > si_after_bad_debt,
        "supply index should recover with new interest: post_bad_debt={}, recovered={}",
        si_after_bad_debt,
        si_recovered
    );
}

// ---------------------------------------------------------------------------
// 5. Bad debt via keeper clean_bad_debt also decreases supply index
// ---------------------------------------------------------------------------

#[test]
fn test_keeper_clean_bad_debt_decreases_supply_index() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(BOB, "ETH", 100.0);

    // Create a position that will become bad debt.
    t.supply(ALICE, "USDC", 8.0); // $8 collateral.
    t.borrow(ALICE, "ETH", 0.002); // $4 debt.

    let (si_before, _) = get_indexes(&t, "ETH");

    // Crash USDC so collateral drops below $5.
    t.set_price("USDC", usd_cents(5)); // $0.40 collateral.

    // Clean the bad debt through the keeper, not liquidation.
    let account_id = t.resolve_account_id(ALICE);
    t.clean_bad_debt_by_id(account_id);

    let (si_after, _) = get_indexes(&t, "ETH");

    // The supply index must drop.
    assert!(
        si_after < si_before,
        "keeper clean_bad_debt should decrease supply index: before={}, after={}",
        si_before,
        si_after
    );

    // Alice's positions must be gone.
    t.assert_no_positions(ALICE);
}

// ---------------------------------------------------------------------------
// 6. Borrow index is NOT affected by bad debt socialization
// ---------------------------------------------------------------------------

#[test]
fn test_bad_debt_does_not_affect_borrow_index() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10.0);
    t.borrow(ALICE, "ETH", 0.003);

    // Sync first to get clean indexes.
    t.advance_and_sync(days(1));
    let (_, bi_before) = get_indexes(&t, "ETH");

    // Create and resolve bad debt.
    t.set_price("USDC", usd_cents(10));
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.001);

    let (_, bi_after) = get_indexes(&t, "ETH");

    // The borrow index must never decrease; it only rises. A small bump
    // from global_sync during liquidation is fine.
    assert!(
        bi_after >= bi_before,
        "borrow index should never decrease, even during bad debt: before={}, after={}",
        bi_before,
        bi_after
    );
}

// ---------------------------------------------------------------------------
// 7. Quantitative: bad debt amount matches supply index reduction
// ---------------------------------------------------------------------------

#[test]
fn test_bad_debt_reduction_matches_formula() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Large supply keeps the bad-debt effect measurable but small.
    t.supply(BOB, "ETH", 1000.0); // $2M supply.

    // Create a known bad debt.
    t.supply(ALICE, "USDC", 10.0); // $10.
    t.borrow(ALICE, "ETH", 0.003); // $6 debt = 0.003 ETH.

    let bob_balance_before = t.supply_balance(BOB, "ETH");

    // Crash to trigger bad debt.
    t.set_price("USDC", usd_cents(10));
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.001);

    let bob_balance_after = t.supply_balance(BOB, "ETH");
    let bob_loss = bob_balance_before - bob_balance_after;

    // Bob's loss must approximate the bad-debt amount. Bad debt ~ remaining
    // borrow after partial liquidation ~ 0.002 ETH, socialized across 1000
    // ETH of supply. Bob is ~ the sole supplier, so his loss ~ bad debt.
    assert!(
        bob_loss > 0.0 && bob_loss < 0.01,
        "Bob's loss should be small (~ bad debt amount): {:.6} ETH",
        bob_loss
    );
}
