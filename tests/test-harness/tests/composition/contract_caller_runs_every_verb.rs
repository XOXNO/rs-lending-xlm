//! GH-08. One contract, as the top-level caller, drives every user, keeper
//! and delegate verb. The last two tests flip the host to enforcing auth so
//! the runner's own `authorize_as_current_contract` entries are what carries
//! the token pulls, not the harness mock.

use crate::helpers::{borrow_op, key, liquidate_op, repay_op, supply_op, withdraw_op};
use common::types::{PositionMode, SeizeMode};
use script_runner::{
    AccountOp, AssetsOp, DelegateOp, FlashLoanOp, MultiplyOp, NftTransferOp, Op, RecapOp,
    ThresholdOp, LAST_CREATED,
};
use soroban_sdk::{vec, Bytes, Vec};
use test_harness::{
    assert_contract_error, build_aggregator_swap, errors, usd, LendingTest, ALICE, BOB, CAROL,
    HARNESS_SPOKE,
};

const U: i128 = 10_000_000;

fn setup() -> LendingTest {
    let mut t = LendingTest::new().standard_two_asset_dust_disabled();
    t.supply(BOB, "ETH", 100.0);
    t.supply(BOB, "USDC", 100_000.0);
    t.fund_router("ETH", 100.0);
    t.fund_router("USDC", 100_000.0);
    t
}

#[test]
fn a_contract_caller_runs_the_position_verbs_in_one_invocation() {
    let t = setup();
    let runner = t.deploy_script_runner();
    t.fund_runner(&runner, "USDC", 20_000 * U);
    t.fund_runner(&runner, "ETH", U);
    let ops: Vec<Op> = vec![
        &t.env,
        supply_op(&t, 0, "USDC", 10_000 * U),
        borrow_op(&t, LAST_CREATED, "ETH", U / 2, None),
        repay_op(&t, LAST_CREATED, "ETH", U / 2 + 1),
        withdraw_op(&t, LAST_CREATED, "USDC", 5_000 * U, None),
        Op::RenewAccount(AccountOp {
            account_id: LAST_CREATED,
        }),
    ];
    let id = t
        .run_script(&runner, &ops)
        .expect("every verb succeeds from a contract frame");
    assert!(id > 0);
    assert_eq!(t.nft_owner_of(id), runner);
    assert_eq!(t.supply_balance_raw_for(id, "USDC"), 5_000 * U);
    assert_eq!(t.borrow_balance_raw_for(id, "ETH"), 0);
    assert_eq!(t.runner_wallet(&runner, "USDC"), 15_000 * U);
}

#[test]
fn a_contract_caller_liquidates_in_both_seize_modes_and_runs_the_keeper_verbs() {
    let mut t = setup();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("ETH", usd(3_000));
    t.assert_liquidatable(ALICE);
    let victim = t.account_id(ALICE);
    let runner = t.deploy_script_runner();
    t.fund_runner(&runner, "ETH", 10 * U);
    t.fund_runner(&runner, "USDC", 100 * U);
    let usdc = key(&t, "USDC");
    let eth = key(&t, "ETH");
    let ops: Vec<Op> = vec![
        &t.env,
        liquidate_op(&t, victim, "ETH", U / 10, SeizeMode::Transfer),
        liquidate_op(&t, victim, "ETH", U / 10, SeizeMode::Credit(0)),
        Op::UpdateIndexes(AssetsOp {
            assets: vec![&t.env, usdc.clone(), eth.clone()],
        }),
        Op::ClaimRevenue(AssetsOp {
            assets: vec![&t.env, usdc.clone()],
        }),
        Op::UpdateAccountThreshold(ThresholdOp {
            has_risks: false,
            account_ids: vec![&t.env, victim],
        }),
        Op::Recapitalize(RecapOp {
            hub_asset: usdc.clone(),
            amount: 100 * U,
        }),
    ];
    let credited = t
        .run_script(&runner, &ops)
        .expect("permissionless verbs run from a contract frame");
    assert!(credited > 0 && credited != victim);
    assert_eq!(t.nft_owner_of(credited), runner);
    assert!(
        t.supply_balance_raw_for(credited, "USDC") > 0,
        "credit mode landed shares on the runner's account"
    );
    assert!(
        t.runner_wallet(&runner, "USDC") > 0,
        "transfer mode paid the runner in USDC"
    );
}

#[test]
fn a_contract_caller_runs_the_strategy_verbs_and_the_flash_loan() {
    let t = setup();
    let runner = t.deploy_script_runner();
    t.fund_runner(&runner, "USDC", 20_000 * U);
    let receiver = t.deploy_flash_loan_receiver();
    // 0.1 ETH of flash-borrowed debt swaps into 200 USDC, on top of 1000
    // USDC the runner pays in.
    let swap = build_aggregator_swap(&t, "ETH", "USDC", 0, 200 * U);
    let ops: Vec<Op> = vec![
        &t.env,
        Op::FlashLoan(FlashLoanOp {
            asset: key(&t, "USDC"),
            amount: 1_000 * U,
            receiver,
            data: Bytes::new(&t.env),
        }),
        Op::Multiply(MultiplyOp {
            account_id: 0,
            spoke_id: HARNESS_SPOKE,
            collateral: key(&t, "USDC"),
            debt_amount: U / 10,
            debt: key(&t, "ETH"),
            mode: PositionMode::Multiply,
            swap,
            initial_payment: vec![&t.env, (key(&t, "USDC"), 1_000 * U)],
        }),
    ];
    let id = t
        .run_script(&runner, &ops)
        .expect("flash loan and multiply from a contract frame");
    assert_eq!(t.borrow_balance_raw_for(id, "ETH"), U / 10);
    assert_eq!(t.supply_balance_raw_for(id, "USDC"), 1_200 * U);
    assert_eq!(t.runner_wallet(&runner, "USDC"), 19_000 * U);
}

#[test]
fn a_contract_caller_grants_and_revokes_a_delegate_and_moves_its_nft() {
    let mut t = setup();
    let delegate = t.get_or_create_user(CAROL);
    t.ctrl_client().set_position_manager(&delegate, &true);
    let runner = t.deploy_script_runner();
    t.fund_runner(&runner, "USDC", 1_000 * U);
    let bob = t.get_or_create_user(BOB);
    let ops: Vec<Op> = vec![
        &t.env,
        supply_op(&t, 0, "USDC", 1_000 * U),
        Op::AddDelegate(DelegateOp {
            account_id: LAST_CREATED,
            delegate: delegate.clone(),
        }),
        Op::RemoveDelegate(DelegateOp {
            account_id: LAST_CREATED,
            delegate,
        }),
        Op::NftTransfer(NftTransferOp {
            to: bob.clone(),
            token_id: LAST_CREATED,
        }),
    ];
    let id = t
        .run_script(&runner, &ops)
        .expect("delegate verbs and the transfer succeed");
    assert_eq!(t.nft_owner_of(id), bob);
}

#[test]
fn under_enforcing_auth_the_runner_carries_its_own_token_pulls() {
    let t = setup();
    let runner = t.deploy_script_runner();
    t.fund_runner(&runner, "USDC", 2_000 * U);
    t.fund_runner(&runner, "ETH", U);
    let ops: Vec<Op> = vec![
        &t.env,
        supply_op(&t, 0, "USDC", 1_000 * U),
        borrow_op(&t, LAST_CREATED, "ETH", U / 10, None),
        repay_op(&t, LAST_CREATED, "ETH", U / 10 + 1),
        withdraw_op(&t, LAST_CREATED, "USDC", i128::MAX, None),
    ];
    // No mocked auth from here on: only invoker auth and the runner's entries exist.
    t.env.set_auths(&[]);
    let id = t
        .run_script(&runner, &ops)
        .expect("a contract caller needs no signature for its own pulls");
    t.env.mock_all_auths_allowing_non_root_auth();
    assert!(
        !t.account_exists(id),
        "withdraw-all emptied and burned the account"
    );
    assert!(t.runner_wallet(&runner, "USDC") >= 2_000 * U - 1);
}

#[test]
fn under_enforcing_auth_a_stranger_cannot_use_the_runner_to_spend_another_account() {
    let mut t = setup();
    t.supply(ALICE, "USDC", 1_000.0);
    let alice_id = t.account_id(ALICE);
    let runner = t.deploy_script_runner();
    let ops: Vec<Op> = vec![&t.env, withdraw_op(&t, alice_id, "USDC", 1, None)];
    t.env.set_auths(&[]);
    let result = t.run_script(&runner, &ops);
    t.env.mock_all_auths_allowing_non_root_auth();
    assert_contract_error(result.map(|_| ()), errors::NOT_AUTHORIZED);
}
