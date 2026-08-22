use test_harness::{assert_contract_error, errors, hub_asset, LendingTest, ALICE, BOB, CAROL};

#[test]
fn test_claim_revenue_else_branch_when_reserves_fully_drained() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_dust_disabled_all_markets()
        .with_max_utilization_disabled_all_markets()
        .build();

    let accumulator = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    t.set_accumulator(&accumulator);

    t.set_oracle_single_spot("USDC");

    t.supply(ALICE, "USDC", 1_000.0);
    // Undebted supply, so the residual the borrow buffer reserves can still be
    // withdrawn: only borrowing is bounded by it.
    t.supply(CAROL, "USDC", 300.0);
    t.borrow(ALICE, "USDC", 700.0);
    t.advance_time(31_536_000);
    t.update_indexes_for(&["USDC"]);

    let revenue_before_claim = t.snapshot_revenue("USDC");
    assert!(
        revenue_before_claim > 0,
        "fixture must accrue revenue before draining reserves"
    );

    t.supply(BOB, "ETH", 1000.0);
    let usdc = hub_asset(t.resolve_asset("USDC"));
    let res_raw = t.pool_client("USDC").get_reserves(&usdc);
    assert!(
        res_raw > 0,
        "expected positive USDC reserves to drain; got {}",
        res_raw
    );
    let buffer_raw = t.liquidation_buffer_raw("USDC");
    t.borrow_raw(BOB, "USDC", res_raw - buffer_raw);
    let residual = t.pool_client("USDC").get_reserves(&usdc);
    t.withdraw_raw(CAROL, "USDC", residual);

    let res_after_drain = t.pool_client("USDC").get_reserves(&usdc);
    assert_eq!(
        res_after_drain, 0,
        "reserves must be zero to reach the else branch"
    );
    let revenue_pre = t.snapshot_revenue("USDC");
    assert!(
        revenue_pre > 0,
        "revenue must remain positive after the drain"
    );

    let claimed = t.claim_revenue("USDC");

    assert_eq!(claimed, 0, "no reserves => no transfer");

    let revenue_post = t.snapshot_revenue("USDC");
    assert!(
        revenue_post >= revenue_pre,
        "revenue must not shrink when nothing transferred: pre={}, post={}",
        revenue_pre,
        revenue_post
    );

    assert_eq!(
        t.pool_client("USDC").get_reserves(&usdc),
        0,
        "reserves remain zero after a no-op claim"
    );
}

#[test]
fn test_claim_revenue_blocked_when_post_state_insolvent() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_dust_disabled_all_markets()
        .with_max_utilization_disabled_all_markets()
        .build();

    let accumulator = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    t.set_accumulator(&accumulator);
    t.set_oracle_single_spot("USDC");

    t.supply(ALICE, "USDC", 1_000.0);
    t.supply(BOB, "ETH", 10.0);
    t.borrow(BOB, "USDC", 500.0);

    t.advance_time(31_536_000);
    t.update_indexes_for(&["USDC"]);
    let revenue_pre = t.snapshot_revenue("USDC");
    assert!(revenue_pre > 0, "fixture must accrue revenue");

    t.withdraw_all(ALICE, "USDC");

    let result = t.try_claim_revenue("USDC");
    assert_contract_error(result, errors::POOL_INSOLVENT);
}
