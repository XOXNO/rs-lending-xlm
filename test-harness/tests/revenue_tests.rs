extern crate std;

use test_harness::{
    days, errors, eth_preset, usd_cents, usdc_preset, LendingTest, ALICE, BOB, LIQUIDATOR,
};

/// Helper: set the accumulator address (required for claim_revenue).
fn setup_accumulator(t: &LendingTest) {
    let acc = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    t.ctrl_client().set_accumulator(&acc);
}

// ---------------------------------------------------------------------------
// 1. test_claim_revenue_after_interest
// ---------------------------------------------------------------------------

#[test]
fn test_claim_revenue_after_interest() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Set up: supply and borrow to generate interest.
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    let revenue_before = t.snapshot_revenue("ETH");

    // Advance time to accrue interest.
    t.advance_and_sync(days(90));

    let revenue_after = t.snapshot_revenue("ETH");
    assert!(
        revenue_after > revenue_before,
        "revenue should accrue from interest: before={}, after={}",
        revenue_before,
        revenue_after
    );

    // Claim the revenue (requires the accumulator).
    setup_accumulator(&t);
    let claimed = t.claim_revenue("ETH");
    assert!(
        claimed > 0,
        "claimed revenue should be positive, got {}",
        claimed
    );
}

// ---------------------------------------------------------------------------
// 2. test_claim_revenue_after_liquidation
// ---------------------------------------------------------------------------

#[test]
fn test_claim_revenue_after_liquidation() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Alice supplies and borrows near the limit.
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0); // ~$6000 debt

    let revenue_before_liq = t.snapshot_revenue("ETH");

    // Drop USDC to trigger liquidation.
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);

    // Liquidate: generates fees.
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);

    // Advance time so interest accrues on remaining positions.
    t.advance_and_sync(days(30));

    let revenue_after_liq = t.snapshot_revenue("ETH");
    assert!(
        revenue_after_liq > revenue_before_liq,
        "revenue should increase after liquidation: before={}, after={}",
        revenue_before_liq,
        revenue_after_liq
    );
}

// ---------------------------------------------------------------------------
// 3. test_claim_revenue_zero_when_no_activity
// ---------------------------------------------------------------------------

#[test]
fn test_claim_revenue_zero_when_no_activity() {
    let t = LendingTest::new().with_market(usdc_preset()).build();

    // No borrows, no interest, no revenue.
    let revenue = t.snapshot_revenue("USDC");
    assert_eq!(revenue, 0, "revenue should be 0 with no activity");

    // Claim returns 0 (still requires the accumulator).
    setup_accumulator(&t);
    let claimed = t.claim_revenue("USDC");
    assert_eq!(claimed, 0, "claimed revenue should be 0 with no activity");
}

// ---------------------------------------------------------------------------
// 4. test_add_rewards_increases_supply_index
// ---------------------------------------------------------------------------

#[test]
fn test_add_rewards_increases_supply_index() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    // Alice supplies USDC.
    t.supply(ALICE, "USDC", 10_000.0);

    let supply_before = t.supply_balance(ALICE, "USDC");

    // Add rewards to the pool.
    t.add_rewards("USDC", 1_000.0);

    // Supply balance must rise as the supply index rises.
    let supply_after = t.supply_balance(ALICE, "USDC");
    assert!(
        supply_after > supply_before,
        "supply balance should increase after rewards: before={}, after={}",
        supply_before,
        supply_after
    );

    // The increase must be roughly 1000 (the reward amount).
    let increase = supply_after - supply_before;
    assert!(
        increase > 900.0 && increase < 1100.0,
        "increase should be ~1000, got {}",
        increase
    );
}

// ---------------------------------------------------------------------------
// 5. test_add_rewards_rejects_zero
// ---------------------------------------------------------------------------

#[test]
fn test_add_rewards_rejects_zero() {
    let t = LendingTest::new().with_market(usdc_preset()).build();

    // Add 0 rewards via the raw controller call.
    let ctrl = t.ctrl_client();
    let admin = t.admin();
    let asset = t.resolve_market("USDC").asset.clone();

    let rewards = soroban_sdk::vec![&t.env, (asset.clone(), 0i128)];
    let result = ctrl.try_add_rewards(&admin, &rewards);
    match result {
        Err(Ok(err)) => assert_eq!(
            err,
            soroban_sdk::Error::from_contract_error(errors::AMOUNT_MUST_BE_POSITIVE),
            "expected AMOUNT_MUST_BE_POSITIVE but got {:?}",
            err
        ),
        Err(Err(invoke_err)) => {
            panic!("expected contract error, got InvokeError: {:?}", invoke_err)
        },
        _ => panic!("add_rewards with 0 amount should fail"),
    }
}

// ---------------------------------------------------------------------------
// 6. test_revenue_role_required
// ---------------------------------------------------------------------------

#[test]
fn test_revenue_role_required() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    // Create Bob without the REVENUE role.
    let bob_addr = t.get_or_create_user(BOB);

    let ctrl = t.ctrl_client();
    let asset = t.resolve_market("USDC").asset.clone();

    // Bob tries claim_revenue.
    let assets = soroban_sdk::vec![&t.env, asset.clone()];
    let result = ctrl.try_claim_revenue(&bob_addr, &assets);
    assert!(
        result.is_err(),
        "non-revenue user should not be able to claim revenue"
    );

    // Bob tries add_rewards.
    let rewards = soroban_sdk::vec![&t.env, (asset, 100i128)];
    let result = ctrl.try_add_rewards(&bob_addr, &rewards);
    assert!(
        result.is_err(),
        "non-revenue user should not be able to add rewards"
    );
}
