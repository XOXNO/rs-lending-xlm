//! A first `Supply(LAST_CREATED)` resolves to `0` and opens an account; the
//! next `LAST_CREATED` leg must act on that account.

use crate::helpers::{supply_op, withdraw_op};
use script_runner::{Op, LAST_CREATED};
use soroban_sdk::{vec, Vec};
use test_harness::LendingTest;

const U: i128 = 10_000_000;

#[test]
fn a_sentinel_supply_that_opens_an_account_becomes_last_created() {
    let t = LendingTest::new().standard_two_asset_dust_disabled();
    let runner = t.deploy_script_runner();
    t.fund_runner(&runner, "USDC", 1_000 * U);
    let ops: Vec<Op> = vec![
        &t.env,
        supply_op(&t, LAST_CREATED, "USDC", 1_000 * U),
        withdraw_op(&t, LAST_CREATED, "USDC", 400 * U, None),
    ];

    let id = t
        .run_script(&runner, &ops)
        .expect("the withdraw must act on the account the supply opened");

    assert_ne!(id, 0);
    assert_eq!(t.nft_owner_of(id), runner);
    assert_eq!(t.runner_wallet(&runner, "USDC"), 400 * U);
    let (supply, borrow) = t.ctrl_client().get_account_positions(&id);
    assert_eq!((supply.len(), borrow.len()), (1, 0));
}
