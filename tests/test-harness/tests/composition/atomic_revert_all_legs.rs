//! GH-09. A script whose last leg fails commits nothing from the earlier legs.

use crate::helpers::{borrow_op, repay_op, supply_op, withdraw_op, Snapshot};
use script_runner::{Op, LAST_CREATED};
use soroban_sdk::{vec, Vec};
use test_harness::{assert_contract_error, errors, LendingTest, BOB};

const U: i128 = 10_000_000;

#[test]
fn a_failing_last_leg_rolls_back_supply_borrow_and_repay() {
    let mut t = LendingTest::new().standard_two_asset_dust_disabled();
    t.supply(BOB, "ETH", 100.0);
    let runner = t.deploy_script_runner();
    t.fund_runner(&runner, "USDC", 10_000 * U);
    t.fund_runner(&runner, "ETH", U);
    let before = Snapshot::take(&t, &runner, &["USDC", "ETH"], &[]);
    let ops: Vec<Op> = vec![
        &t.env,
        supply_op(&t, 0, "USDC", 10_000 * U),
        borrow_op(&t, LAST_CREATED, "ETH", U / 2, None),
        repay_op(&t, LAST_CREATED, "ETH", U / 4),
        // Withdrawing everything with debt still open fails the solvency gate.
        withdraw_op(&t, LAST_CREATED, "USDC", 0, None),
    ];
    let result = t.run_script(&runner, &ops);
    assert_contract_error(result.map(|_| ()), errors::INSUFFICIENT_COLLATERAL);
    let after = Snapshot::take(&t, &runner, &["USDC", "ETH"], &[]);
    assert_eq!(before, after, "a partial script must leave no trace");
    let next_id = t.create_account(BOB);
    assert!(
        next_id > 0,
        "the NFT id counter is part of the reverted state; the next mint must still work"
    );
}
