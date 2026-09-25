use soroban_sdk::testutils::Ledger;
use soroban_sdk::{panic_with_error, token, Error};
use test_harness::{build_aggregator_swap, hub_asset, LendingTest, ALICE, BOB};

/// An exact collateral withdrawal closes the supply leg during simulation, but
/// five seconds of interest leave a remainder to withdraw during inclusion.
#[test]
fn rdwc_close_preserves_recipient_footprint_when_accrual_adds_a_close_leg() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply("BOOT", "USDC", 200_000.0);
    t.supply(ALICE, "USDC", 400.0);
    t.borrow(ALICE, "ETH", 0.01);
    t.supply(BOB, "ETH", 1_000.0);
    t.borrow(BOB, "USDC", 100_000.0);
    t.fund_router("ETH", 1.0);
    let caller = t.users[ALICE].address.clone();
    let account_id = t.resolve_account_id(ALICE);
    let collateral = hub_asset(t.resolve_asset("USDC"));
    let debt = hub_asset(t.resolve_asset("ETH"));
    let swap = build_aggregator_swap(&t, "USDC", "ETH", 4_000_000_000, 200_000);
    let frame = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    t.env
        .register_at(&caller, test_harness::mock_reflector::MockReflector, ());
    t.env.set_auths(&[]);
    let rollback = Error::from_contract_error(999);
    let before = token::Client::new(&t.env, &collateral.asset).balance(&caller);

    // Keep recording and enforcing in one test frame: top-level SDK calls would
    // otherwise reset the footprint. A failed nested frame rolls back simulation.
    t.env.as_contract(&frame, || {
        let simulated = t.env.try_as_contract::<_, Error>(&caller, || {
            t.ctrl_client().repay_debt_with_collateral(
                &caller,
                &account_id,
                &collateral,
                &4_000_000_000,
                &debt,
                &swap,
                &true,
            );
            assert!(!t.ctrl_client().account_exists(&account_id));
            panic_with_error!(&t.env, rollback);
        });
        assert_eq!(simulated, Err(Ok(rollback)));
        t.env.ledger().with_mut(|info| {
            info.timestamp += 5;
            info.sequence_number += 1;
        });
        t.env.host().switch_to_enforcing_storage().unwrap();
        t.env.as_contract(&caller, || {
            t.ctrl_client().repay_debt_with_collateral(
                &caller,
                &account_id,
                &collateral,
                &4_000_000_000,
                &debt,
                &swap,
                &true,
            )
        });
        assert!(!t.ctrl_client().account_exists(&account_id));
        assert!(token::Client::new(&t.env, &collateral.asset).balance(&caller) > before);
    });
}

/// Net settlement remains an exit when an administrator lowers the cap below
/// current utilization; its zero-share footprint reservation moves no funds.
#[test]
fn same_market_close_above_utilization_cap_keeps_net_settlement_reachable() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply("BOOT", "USDC", 200_000.0);
    t.supply(ALICE, "USDC", 400.0);
    t.supply(ALICE, "ETH", 1.0);
    t.borrow(ALICE, "USDC", 400.0);
    t.supply(BOB, "ETH", 1_000.0);
    t.borrow(BOB, "USDC", 180_000.0);
    let key = hub_asset(t.resolve_asset("USDC"));
    let mut model = t
        .pool_client("USDC")
        .get_sync_data(&key)
        .params
        .rate_model_view();
    model.max_utilization = controller::constants::RAY * 8 / 10;
    t.upgrade_pool_params("USDC", model);
    let cash_before = t.pool_client("USDC").get_sync_data(&key).state.cash;
    let account_id = t.resolve_account_id(ALICE);
    t.repay_debt_with_collateral(
        ALICE,
        "USDC",
        400.0,
        "USDC",
        &soroban_sdk::Bytes::new(&t.env),
        true,
    );
    assert!(!t.ctrl_client().account_exists(&account_id));
    assert_eq!(
        t.pool_client("USDC").get_sync_data(&key).state.cash,
        cash_before
    );
}
