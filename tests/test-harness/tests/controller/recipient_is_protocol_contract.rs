//! GH-17. A borrow or withdraw addressed to the pool or the controller
//! strands the tokens: the pool debits cash without its balance moving, and
//! the controller holds funds no balance-delta measurement can ever claim.
//! Both recipients are rejected before any transfer, with the same error the
//! flash-position receiver check uses.

use soroban_sdk::vec;
use test_harness::{
    assert_contract_error, errors, hub_asset, map_try_ok_unit, LendingTest, ALICE, BOB,
};

const U: i128 = 10_000_000;

fn setup() -> LendingTest {
    let mut t = LendingTest::new().standard_two_asset_dust_disabled();
    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10_000.0);
    t
}

#[test]
fn borrow_to_the_pool_is_rejected_before_cash_moves() {
    let mut t = setup();
    let id = t.account_id(ALICE);
    let pool = t.get_pool_address("ETH");
    let cash_before = t.pool_reserves("ETH");
    let alice = t.get_or_create_user(ALICE);
    let leg = vec![&t.env, (hub_asset(t.resolve_asset("ETH")), U)];
    let result = t.ctrl_client().try_borrow(&alice, &id, &leg, &Some(pool));
    assert_contract_error(map_try_ok_unit(result), errors::INVALID_FLASHLOAN_RECEIVER);
    assert_eq!(t.pool_reserves("ETH"), cash_before);
}

#[test]
fn borrow_to_the_controller_is_rejected() {
    let mut t = setup();
    let id = t.account_id(ALICE);
    let controller = t.controller_address();
    let alice = t.get_or_create_user(ALICE);
    let leg = vec![&t.env, (hub_asset(t.resolve_asset("ETH")), U)];
    let result = t
        .ctrl_client()
        .try_borrow(&alice, &id, &leg, &Some(controller));
    assert_contract_error(map_try_ok_unit(result), errors::INVALID_FLASHLOAN_RECEIVER);
}

#[test]
fn withdraw_to_the_pool_or_the_controller_is_rejected() {
    let mut t = setup();
    let id = t.account_id(ALICE);
    let alice = t.get_or_create_user(ALICE);
    let leg = vec![&t.env, (hub_asset(t.resolve_asset("USDC")), U)];
    for bad in [t.get_pool_address("USDC"), t.controller_address()] {
        let flat: Result<(), soroban_sdk::Error> =
            match t.ctrl_client().try_withdraw(&alice, &id, &leg, &Some(bad)) {
                Ok(_) => Ok(()),
                Err(e) => Err(e.expect("expected a contract error, got an InvokeError")),
            };
        assert_contract_error(flat, errors::INVALID_FLASHLOAN_RECEIVER);
    }
    assert_eq!(t.supply_balance_raw(ALICE, "USDC"), 10_000 * U);
}
