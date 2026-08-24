use test_harness::{
    assert_contract_error, errors, map_try_ok_unit, usdc_preset, LendingTest, ALICE, BOB,
};

fn fresh() -> LendingTest {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    let _ = t.get_or_create_user(ALICE);
    let _ = t.get_or_create_user(BOB);
    t
}

#[test]
fn test_transfer_and_accept_ownership_completes() {
    let t = fresh();
    let ctrl = t.ctrl_client();
    let new_owner = t.users.get(ALICE).unwrap().address.clone();

    let ledger_seq = t.env.ledger().sequence();
    ctrl.transfer_ownership(&new_owner, &(ledger_seq + 1000));

    t.env.mock_all_auths();
    ctrl.accept_ownership();

    ctrl.pause();
    assert_eq!(t.env.auths()[0].0, new_owner);
}

#[test]
fn test_transfer_ownership_with_zero_ttl_cancels_pending() {
    let t = fresh();
    let ctrl = t.ctrl_client();
    let candidate = t.users.get(ALICE).unwrap().address.clone();

    let ledger_seq = t.env.ledger().sequence();
    ctrl.transfer_ownership(&candidate, &(ledger_seq + 500));

    ctrl.transfer_ownership(&candidate, &0u32);

    let result = map_try_ok_unit(ctrl.try_accept_ownership());
    assert_contract_error(result, errors::NO_PENDING_TRANSFER);
}

#[test]
fn test_transfer_ownership_to_self_keeps_owner() {
    let t = fresh();
    let ctrl = t.ctrl_client();
    let admin = t.admin();

    let ledger_seq = t.env.ledger().sequence();
    ctrl.transfer_ownership(&admin, &(ledger_seq + 1000));
    t.env.mock_all_auths();
    ctrl.accept_ownership();

    ctrl.pause();
    assert_eq!(t.env.auths()[0].0, admin);
}

#[test]
fn test_pause_unpause_round_trip() {
    let mut t = fresh();
    t.pause();
    assert_contract_error(t.try_supply(ALICE, "USDC", 1.0), errors::CONTRACT_PAUSED);

    t.unpause();
    t.supply(ALICE, "USDC", 1.0);
    t.assert_supply_near(ALICE, "USDC", 1.0, 0.001);
}

#[test]
fn test_app_version_defaults_to_initial() {
    let t = fresh();
    assert_eq!(t.ctrl_client().get_app_version(), 1);
}

#[test]
fn test_migrate_bumps_version_when_strictly_greater() {
    let t = fresh();
    let ctrl = t.ctrl_client();
    ctrl.migrate(&2);
    assert_eq!(ctrl.get_app_version(), 2);
    ctrl.migrate(&5);
    assert_eq!(ctrl.get_app_version(), 5);
}

#[test]
#[should_panic(expected = "Error(Contract, #34)")]
fn test_migrate_rejects_equal_version() {
    let t = fresh();
    let ctrl = t.ctrl_client();

    ctrl.migrate(&1);
}

#[test]
#[should_panic(expected = "Error(Contract, #34)")]
fn test_migrate_rejects_lower_version() {
    let t = fresh();
    let ctrl = t.ctrl_client();
    ctrl.migrate(&3);

    ctrl.migrate(&2);
}
