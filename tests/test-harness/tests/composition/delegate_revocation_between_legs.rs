//! GH-12. A delegate contract keeps only the permissionless verbs after the
//! owner removes it, and loses everything the moment governance deactivates
//! it as a position manager, even while the stored grant still lists it.

use crate::helpers::{borrow_op, repay_op, withdraw_op};
use soroban_sdk::vec;
use test_harness::{assert_contract_error, errors, LendingTest, ALICE, BOB};

const U: i128 = 10_000_000;

#[test]
fn a_removed_delegate_can_still_repay_but_cannot_borrow_or_withdraw() {
    let mut t = LendingTest::new().standard_two_asset_dust_disabled();
    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10_000.0);
    let account = t.account_id(ALICE);
    let runner = t.deploy_script_runner();
    t.fund_runner(&runner, "ETH", U);
    let alice = t.get_or_create_user(ALICE);
    t.ctrl_client().set_position_manager(&runner, &true);
    t.ctrl_client().add_delegate(&alice, &account, &runner);

    t.run_script(
        &runner,
        &vec![&t.env, borrow_op(&t, account, "ETH", U / 10, None)],
    )
    .expect("a live delegate borrows");

    t.ctrl_client().remove_delegate(&alice, &account, &runner);
    t.run_script(&runner, &vec![&t.env, repay_op(&t, account, "ETH", U / 20)])
        .expect("repay is permissionless");
    assert_contract_error(
        t.run_script(
            &runner,
            &vec![&t.env, borrow_op(&t, account, "ETH", 1, None)],
        )
        .map(|_| ()),
        errors::NOT_AUTHORIZED,
    );
    assert_contract_error(
        t.run_script(
            &runner,
            &vec![&t.env, withdraw_op(&t, account, "USDC", 1, None)],
        )
        .map(|_| ()),
        errors::NOT_AUTHORIZED,
    );
}

#[test]
fn deactivating_the_manager_kills_a_stored_grant_immediately() {
    let mut t = LendingTest::new().standard_two_asset_dust_disabled();
    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10_000.0);
    let account = t.account_id(ALICE);
    let runner = t.deploy_script_runner();
    let alice = t.get_or_create_user(ALICE);
    t.ctrl_client().set_position_manager(&runner, &true);
    t.ctrl_client().add_delegate(&alice, &account, &runner);
    t.ctrl_client().set_position_manager(&runner, &false);
    assert_contract_error(
        t.run_script(
            &runner,
            &vec![&t.env, borrow_op(&t, account, "ETH", U / 10, None)],
        )
        .map(|_| ()),
        errors::NOT_AUTHORIZED,
    );
    t.ctrl_client().set_position_manager(&runner, &true);
    t.run_script(
        &runner,
        &vec![&t.env, borrow_op(&t, account, "ETH", U / 10, None)],
    )
    .expect("reactivation re-arms the still-stored grant");
}
