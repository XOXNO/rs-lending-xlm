use test_harness::{eth_preset, usdc_preset, LendingTest, ALICE, BOB};

#[test]
fn test_pool_claim_revenue_burns_supplied_ray_coverage() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    // Setup accumulator for revenue claims
    let accumulator = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    t.set_accumulator(&accumulator);

    // Bypass TWAP to avoid #211 (AccumulatorNotSet)
    t.set_exchange_source("USDC", common::types::ExchangeSource::SpotOnly); // 0 = SPOT ONLY

    // 1. Supply some liquidty
    t.supply(ALICE, "USDC", 1000.0);
    t.supply(BOB, "USDC", 1000.0);

    // 2. Borrow to generate interest
    t.borrow(ALICE, "USDC", 500.0);

    // 3. Jump forward in time 1 year
    t.advance_time(31_536_000); // 1 year

    // 4. Update indexes to accrue revenue
    t.update_indexes_for(&["USDC"]);

    // Check revenue exists (snapshot_revenue uses pool's internal state)
    let rev = t.snapshot_revenue("USDC");
    assert!(rev > 0, "Expected some revenue after 1 year");

    // 5. Claim revenue. This should hit pool/src/lib.rs:401
    let claimed = t.claim_revenue("USDC");
    assert!(claimed > 0, "Should have claimed some revenue");

    // Check that revenue was indeed burned from the pool
    let rev_after = t.snapshot_revenue("USDC");
    assert_eq!(rev_after, 0);
}

#[test]
fn test_pool_claim_revenue_proportional_burn_when_reserves_low() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let accumulator = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    t.set_accumulator(&accumulator);
    t.set_exchange_source("USDC", common::types::ExchangeSource::SpotOnly);

    t.supply(ALICE, "USDC", 1000.0);
    t.borrow(ALICE, "USDC", 700.0);

    t.advance_time(31_536_000);
    t.update_indexes_for(&["USDC"]);

    // 6. Force reserves to be low for the pool
    // Bob supplies ETH so he can borrow USDC without increasing USDC reserves.
    t.supply(BOB, "ETH", 1000.0); // $2M collateral

    let res = t.pool_reserves("USDC");
    t.borrow(BOB, "USDC", res - 1.0);

    // Now reserves are near 0.
    // Ensure revenue accrued is greater than reserves.
    let rev = t.snapshot_revenue("USDC");
    let res_raw = t.pool_client("USDC").reserves();
    assert!(
        rev > res_raw,
        "Revenue {} must be > reserves {} to hit proportional burn",
        rev,
        res_raw
    );

    let claimed = t.claim_revenue("USDC");

    // For coverage, we just need this to run.
    assert!(claimed > 0);
    assert_eq!(claimed, res_raw); // Should be capped at reserves

    // Verify that revenue was reduced but not cleared (proportional burn)
    let rev_remaining = t.snapshot_revenue("USDC");
    assert!(rev_remaining > 0);
}
