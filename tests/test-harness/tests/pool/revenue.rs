use test_harness::{
    days, errors, eth_preset, hub_asset, usd_cents, usdc_preset, LendingTest, ALICE, BOB,
    LIQUIDATOR,
};

fn setup_accumulator(t: &LendingTest) {
    let acc = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    t.ctrl_client().set_accumulator(&acc);
}
#[test]
fn test_claim_revenue_after_interest() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(BOB, "ETH", 100.0);

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    let revenue_before = t.snapshot_revenue("ETH");

    t.advance_and_sync(days(90));

    let revenue_after = t.snapshot_revenue("ETH");
    assert!(
        revenue_after > revenue_before,
        "revenue should accrue from interest: before={}, after={}",
        revenue_before,
        revenue_after
    );

    setup_accumulator(&t);
    let claimed = t.claim_revenue("ETH");
    assert!(
        claimed > 0,
        "claimed revenue should be positive, got {}",
        claimed
    );
}

#[test]
fn test_claim_revenue_routes_through_controller_to_accumulator() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(BOB, "ETH", 100.0);

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);
    t.advance_and_sync(days(90));

    let accumulator = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    t.ctrl_client().set_accumulator(&accumulator);

    let asset = t.resolve_market("ETH").asset.clone();
    let pool_addr = t.resolve_market("ETH").pool.clone();
    let controller_addr = t.controller_address();
    let tok = soroban_sdk::token::Client::new(&t.env, &asset);

    let pool_before = tok.balance(&pool_addr);
    let controller_before = tok.balance(&controller_addr);
    let accumulator_before = tok.balance(&accumulator);

    let claimed = t.claim_revenue("ETH");
    assert!(claimed > 0, "expected non-zero claim; got {}", claimed);

    let pool_after = tok.balance(&pool_addr);
    let controller_after = tok.balance(&controller_addr);
    let accumulator_after = tok.balance(&accumulator);

    assert_eq!(
        controller_before, controller_after,
        "controller must not retain claimed tokens between hops"
    );
    assert_eq!(
        accumulator_after - accumulator_before,
        claimed,
        "accumulator must receive the full claimed amount"
    );
    assert_eq!(
        pool_before - pool_after,
        claimed,
        "pool must release exactly the claimed amount"
    );
}
#[test]
fn test_claim_revenue_after_liquidation() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(BOB, "ETH", 100.0);

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);

    let revenue_before_liq = t.snapshot_revenue("ETH");

    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);

    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);

    t.advance_and_sync(days(30));

    let revenue_after_liq = t.snapshot_revenue("ETH");
    assert!(
        revenue_after_liq > revenue_before_liq,
        "post-liq + interest accrual must lift revenue: before={}, after_30d={}",
        revenue_before_liq,
        revenue_after_liq
    );

    let accumulator = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    t.ctrl_client().set_accumulator(&accumulator);

    let asset = t.resolve_market("ETH").asset.clone();
    let pool_addr = t.resolve_market("ETH").pool.clone();
    let controller_addr = t.controller_address();
    let tok = soroban_sdk::token::Client::new(&t.env, &asset);

    let pool_before = tok.balance(&pool_addr);
    let controller_before = tok.balance(&controller_addr);
    let accumulator_before = tok.balance(&accumulator);

    let claimed = t.claim_revenue("ETH");
    assert!(claimed > 0, "expected non-zero claim; got {}", claimed);

    let pool_after = tok.balance(&pool_addr);
    let controller_after = tok.balance(&controller_addr);
    let accumulator_after = tok.balance(&accumulator);

    assert_eq!(
        controller_before, controller_after,
        "controller must not retain claimed tokens between hops"
    );
    assert_eq!(
        accumulator_after - accumulator_before,
        claimed,
        "accumulator must receive the full claimed amount"
    );
    assert_eq!(
        pool_before - pool_after,
        claimed,
        "pool must release exactly the claimed amount"
    );
}
#[test]
fn test_claim_revenue_zero_when_no_activity() {
    let t = LendingTest::new().with_market(usdc_preset()).build();

    let revenue = t.snapshot_revenue("USDC");
    assert_eq!(revenue, 0, "revenue should be 0 with no activity");

    setup_accumulator(&t);
    let claimed = t.claim_revenue("USDC");
    assert_eq!(claimed, 0, "claimed revenue should be 0 with no activity");
}
#[test]
fn test_add_rewards_increases_supply_index() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 10_000.0);

    let supply_before = t.supply_balance(ALICE, "USDC");

    t.add_rewards("USDC", 1_000.0);

    let supply_after = t.supply_balance(ALICE, "USDC");
    assert!(
        supply_after > supply_before,
        "supply balance should increase after rewards: before={}, after={}",
        supply_before,
        supply_after
    );

    let increase = supply_after - supply_before;
    assert!(
        increase > 900.0 && increase < 1100.0,
        "increase should be ~1000, got {}",
        increase
    );
}
#[test]
fn test_add_rewards_rejects_zero() {
    let t = LendingTest::new().with_market(usdc_preset()).build();

    let ctrl = t.ctrl_client();
    let admin = t.admin();
    let asset = t.resolve_market("USDC").asset.clone();

    let rewards = soroban_sdk::vec![&t.env, (hub_asset(asset.clone()), 0i128)];
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
        }
        _ => panic!("add_rewards with 0 amount should fail"),
    }
}
#[test]
fn test_permissionless_revenue_endpoints() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    let bob_addr = t.get_or_create_user(BOB);

    t.supply(ALICE, "USDC", 10_000.0);
    t.resolve_market("USDC")
        .token_admin
        .mint(&bob_addr, &100i128);

    let ctrl = t.ctrl_client();
    let asset = t.resolve_market("USDC").asset.clone();

    t.env.mock_all_auths();
    let assets = soroban_sdk::vec![&t.env, hub_asset(asset.clone())];
    assert!(
        ctrl.try_claim_revenue(&bob_addr, &assets).is_ok(),
        "any signed caller may claim_revenue"
    );

    let rewards = soroban_sdk::vec![&t.env, (hub_asset(asset), 100i128)];
    assert!(
        ctrl.try_add_rewards(&bob_addr, &rewards).is_ok(),
        "any signed caller may add_rewards"
    );
}
