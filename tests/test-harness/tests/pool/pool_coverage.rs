use test_harness::{hub_asset, usdc_preset, LendingTest, ALICE, BOB};

#[test]
fn test_pool_claim_revenue_burns_supplied_ray_coverage() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    let accumulator = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    t.set_accumulator(&accumulator);

    t.set_oracle_single_spot("USDC");

    t.supply(ALICE, "USDC", 1000.0);
    t.supply(BOB, "USDC", 1000.0);

    t.borrow(ALICE, "USDC", 500.0);

    t.advance_time(31_536_000);

    t.update_indexes_for(&["USDC"]);

    let rev = t.snapshot_revenue("USDC");
    assert!(rev > 0, "Expected some revenue after 1 year");

    let asset = t.resolve_market("USDC").asset.clone();
    let pool_addr = t.resolve_market("USDC").pool.clone();
    let tok = soroban_sdk::token::Client::new(&t.env, &asset);
    let pool_before = tok.balance(&pool_addr);
    let acc_before = tok.balance(&accumulator);

    let claimed = t.claim_revenue("USDC");
    assert_eq!(
        claimed, rev,
        "full-burn branch must claim the entire accrued revenue"
    );

    let rev_after = t.snapshot_revenue("USDC");
    assert_eq!(rev_after, 0);

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
        .standard_two_asset()
        .with_max_utilization_disabled_all_markets()
        .build();

    let accumulator = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    t.set_accumulator(&accumulator);
    t.set_oracle_single_spot("USDC");

    t.supply(ALICE, "USDC", 1000.0);
    t.borrow(ALICE, "USDC", 700.0);

    t.advance_time(31_536_000);
    t.update_indexes_for(&["USDC"]);

    t.supply(BOB, "ETH", 1000.0);

    let usdc_key = hub_asset(t.resolve_asset("USDC"));
    let res = t.pool_client("USDC").get_reserves(&usdc_key);
    // Down to the borrow buffer, the lowest an ordinary borrow can take reserves.
    t.borrow_raw(BOB, "USDC", res - t.liquidation_buffer_raw("USDC"));

    // Cash is now pinned at the buffer while near-full utilization keeps accruing
    // revenue, which is what drives revenue past reserves.
    t.advance_time(31_536_000 * 4);
    t.update_indexes_for(&["USDC"]);

    let rev = t.snapshot_revenue("USDC");
    let usdc = t.resolve_asset("USDC");
    let res_raw = t.pool_client("USDC").get_reserves(&hub_asset(usdc));
    assert!(
        rev > res_raw,
        "Revenue {} must be > reserves {} to hit proportional burn",
        rev,
        res_raw
    );

    let asset = t.resolve_market("USDC").asset.clone();
    let pool_addr = t.resolve_market("USDC").pool.clone();
    let tok = soroban_sdk::token::Client::new(&t.env, &asset);
    let pool_before = tok.balance(&pool_addr);
    let acc_before = tok.balance(&accumulator);

    let claimed = t.claim_revenue("USDC");

    assert!(
        claimed > 0,
        "proportional burn branch must claim positive revenue"
    );
    assert_eq!(claimed, res_raw, "claim must be capped at pool reserves");

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

    let rev_remaining = t.snapshot_revenue("USDC");
    assert!(rev_remaining > 0);
}
