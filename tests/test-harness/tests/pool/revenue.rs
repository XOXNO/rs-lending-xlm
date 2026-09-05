use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
use soroban_sdk::IntoVal;
use test_harness::{days, hub_asset, usd_cents, usdc_preset, LendingTest, ALICE, BOB, LIQUIDATOR};

fn setup_accumulator(t: &LendingTest) {
    let acc = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    t.ctrl_client().set_accumulator(&acc);
}
#[test]
fn test_claim_revenue_after_interest() {
    let mut t = LendingTest::new().standard_two_asset().build();

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
    let mut t = LendingTest::new().standard_two_asset().build();

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
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(BOB, "ETH", 100.0);

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);

    let revenue_before_liq = t.snapshot_revenue("ETH");

    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);

    let usdc_revenue_before = t.snapshot_revenue("USDC");
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);

    // Isolated from interest: the seizure's protocol fee must credit the
    // collateral pool's revenue immediately, before any accrual runs.
    assert!(
        t.snapshot_revenue("USDC") > usdc_revenue_before,
        "the liquidation protocol fee alone must lift USDC revenue"
    );

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
fn test_permissionless_revenue_endpoints() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);
    t.advance_and_sync(days(90));

    let accumulator = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    t.ctrl_client().set_accumulator(&accumulator);

    let bob_addr = t.get_or_create_user(BOB);
    let asset = t.resolve_market("ETH").asset.clone();
    let assets = soroban_sdk::vec![&t.env, hub_asset(asset.clone())];
    let expected = t.snapshot_revenue("ETH");
    assert!(expected > 0, "fixture must accrue revenue, got {expected}");

    let tok = soroban_sdk::token::Client::new(&t.env, &asset);
    let acc_before = tok.balance(&accumulator);
    let bob_before = tok.balance(&bob_addr);

    // Permissionless is not unsigned: with no auth entries the caller's own
    // `require_auth` must still refuse.
    t.env.set_auths(&[]);
    assert!(
        t.ctrl_client()
            .try_claim_revenue(&bob_addr, &assets)
            .is_err(),
        "claim_revenue must still require the caller's own signature"
    );

    // Sign as Bob and as nobody else. The builder's blanket
    // `mock_all_auths_allowing_non_root_auth` is what made the old version
    // vacuous: under it an admin gate on `claim_revenue` would still pass.
    t.env.mock_auths(&[MockAuth {
        address: &bob_addr,
        invoke: &MockAuthInvoke {
            contract: &t.controller,
            fn_name: "claim_revenue",
            args: (bob_addr.clone(), assets.clone()).into_val(&t.env),
            sub_invokes: &[],
        },
    }]);
    let claimed = t
        .ctrl_client()
        .claim_revenue(&bob_addr, &assets)
        .get(0)
        .unwrap();
    t.env.mock_all_auths_allowing_non_root_auth();

    assert_eq!(
        claimed, expected,
        "a non-admin caller sweeps the full accrued revenue"
    );
    assert_eq!(
        tok.balance(&accumulator) - acc_before,
        claimed,
        "the accumulator, not the caller, receives the revenue"
    );
    assert_eq!(
        tok.balance(&bob_addr),
        bob_before,
        "the caller keeps nothing for driving the sweep"
    );
    assert_eq!(
        t.snapshot_revenue("ETH"),
        0,
        "revenue is zeroed after the sweep"
    );
}
