//! GH-11. Ownership is read live from the NFT, so a transfer between two legs
//! of one script revokes the runner's authority on the very next leg.

use crate::helpers::{liquidate_op, supply_op, withdraw_op};
use common::types::SeizeMode;
use script_runner::{NftTransferOp, Op, LAST_CREATED};
use soroban_sdk::{vec, Vec};
use test_harness::{assert_contract_error, errors, usd, LendingTest, ALICE, BOB};

const U: i128 = 10_000_000;

#[test]
fn a_withdraw_after_an_in_script_transfer_is_rejected() {
    let mut t = LendingTest::new().standard_two_asset_dust_disabled();
    let runner = t.deploy_script_runner();
    t.fund_runner(&runner, "USDC", 1_000 * U);
    let bob = t.get_or_create_user(BOB);
    let ops: Vec<Op> = vec![
        &t.env,
        supply_op(&t, 0, "USDC", 1_000 * U),
        Op::NftTransfer(NftTransferOp {
            to: bob,
            token_id: LAST_CREATED,
        }),
        withdraw_op(&t, LAST_CREATED, "USDC", 1, None),
    ];
    assert_contract_error(
        t.run_script(&runner, &ops).map(|_| ()),
        errors::NOT_AUTHORIZED,
    );
}

#[test]
fn a_credit_liquidation_into_an_account_just_transferred_away_is_rejected() {
    let mut t = LendingTest::new().standard_two_asset_dust_disabled();
    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("ETH", usd(3_000));
    let victim = t.account_id(ALICE);
    let runner = t.deploy_script_runner();
    t.fund_runner(&runner, "USDC", 1_000 * U);
    t.fund_runner(&runner, "ETH", U);
    let bob = t.get_or_create_user(BOB);
    // Open a receiver account, give it away, then try to credit seized shares into it.
    let ops: Vec<Op> = vec![
        &t.env,
        supply_op(&t, 0, "USDC", 1_000 * U),
        Op::NftTransfer(NftTransferOp {
            to: bob,
            token_id: LAST_CREATED,
        }),
    ];
    let receiver_id = t.run_script(&runner, &ops).expect("open and hand over");
    let liquidate: Vec<Op> = vec![
        &t.env,
        liquidate_op(&t, victim, "ETH", U / 10, SeizeMode::Credit(receiver_id)),
    ];
    assert_contract_error(
        t.run_script(&runner, &liquidate).map(|_| ()),
        errors::NOT_AUTHORIZED,
    );
}
