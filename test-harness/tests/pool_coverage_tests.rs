use test_harness::{eth_preset, usdc_preset, LendingTest, ALICE, BOB};

#[test]
fn test_pool_claim_revenue_burns_supplied_ray_coverage() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    // Set up the accumulator for revenue claims.
    let accumulator = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    t.set_accumulator(&accumulator);

    // Bypass TWAP to avoid #211 (OracleError::NoAccumulator).
    t.set_exchange_source("USDC", common::types::ExchangeSource::SpotOnly); // 0 = SPOT ONLY

    // 1. Supply some liquidity.
    t.supply(ALICE, "USDC", 1000.0);
    t.supply(BOB, "USDC", 1000.0);

    // 2. Borrow to generate interest.
    t.borrow(ALICE, "USDC", 500.0);

    // 3. Jump forward 1 year.
    t.advance_time(31_536_000); // 1 year

    // 4. Update indexes to accrue revenue.
    t.update_indexes_for(&["USDC"]);

    // Check that revenue exists (snapshot_revenue reads the pool's internal
    // state).
    let rev = t.snapshot_revenue("USDC");
    assert!(rev > 0, "Expected some revenue after 1 year");

    // Snapshot pool and accumulator balances right before the claim so we
    // can pin the token flow, not just the revenue accumulator state.
    let asset = t.resolve_market("USDC").asset.clone();
    let pool_addr = t.resolve_market("USDC").pool.clone();
    let tok = soroban_sdk::token::Client::new(&t.env, &asset);
    let pool_before = tok.balance(&pool_addr);
    let acc_before = tok.balance(&accumulator);

    // 5. Claim revenue. This must hit pool/src/lib.rs:401.
    let claimed = t.claim_revenue("USDC");
    assert!(claimed > 0, "Should have claimed some revenue");

    // Verify the pool burned the revenue.
    let rev_after = t.snapshot_revenue("USDC");
    assert_eq!(rev_after, 0);

    // Verify the token flow: pool released exactly `claimed`, and the
    // accumulator received exactly `claimed`.
    let pool_after = tok.balance(&pool_addr);
    let acc_after = tok.balance(&accumulator);
    assert_eq!(
        pool_before - pool_after,
        claimed,
        "pool must release exactly the claimed amount"
    );
    assert_eq!(
        acc_after - acc_before,
        claimed,
        "accumulator must receive exactly the claimed amount"
    );
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

    // 6. Force the pool's reserves low.
    // Bob supplies ETH so he can borrow USDC without raising USDC reserves.
    t.supply(BOB, "ETH", 1000.0); // $2M collateral

    let res = t.pool_reserves("USDC");
    t.borrow(BOB, "USDC", res - 1.0);

    // Reserves are now near 0. Ensure accrued revenue exceeds reserves.
    let rev = t.snapshot_revenue("USDC");
    let res_raw = t.pool_client("USDC").reserves();
    assert!(
        rev > res_raw,
        "Revenue {} must be > reserves {} to hit proportional burn",
        rev,
        res_raw
    );

    // Snapshot pool and accumulator balances after the reserve drain but
    // before the claim, so we can pin the token flow on this branch.
    let asset = t.resolve_market("USDC").asset.clone();
    let pool_addr = t.resolve_market("USDC").pool.clone();
    let tok = soroban_sdk::token::Client::new(&t.env, &asset);
    let pool_before = tok.balance(&pool_addr);
    let acc_before = tok.balance(&accumulator);

    let claimed = t.claim_revenue("USDC");

    // For coverage, this only needs to run.
    assert!(claimed > 0);
    assert_eq!(claimed, res_raw); // Capped at reserves.

    // Verify the token flow on the proportional-burn branch: pool released
    // exactly `claimed`, and the accumulator received exactly `claimed`.
    let pool_after = tok.balance(&pool_addr);
    let acc_after = tok.balance(&accumulator);
    assert_eq!(
        pool_before - pool_after,
        claimed,
        "pool must release exactly the claimed amount"
    );
    assert_eq!(
        acc_after - acc_before,
        claimed,
        "accumulator must receive exactly the claimed amount"
    );

    // Verify the proportional burn reduced but did not clear the revenue.
    let rev_remaining = t.snapshot_revenue("USDC");
    assert!(rev_remaining > 0);
}
