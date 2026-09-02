//! GH-15. A supplier can leave, trigger the permissionless clean-up, and come
//! back in one invocation, dodging its share of the write-down. The exit
//! size is bounded by `max_utilization`: past it the whole script reverts.

use crate::helpers::{supply_op, withdraw_op};
use script_runner::{AccountOp, Op};
use soroban_sdk::{vec, Vec};
use test_harness::{assert_contract_error, errors, usd_cents, LendingTest, ALICE, BOB, CAROL};

const U: i128 = 10_000_000;

struct Seed {
    t: LendingTest,
    runner: soroban_sdk::Address,
    runner_account: u64,
    victim: u64,
}

/// `whale_borrow_eth` is drawn by CAROL at the pre-crash USDC price so the
/// utilization bound, not liquidity, is what a full exit breaks.
fn seed(max_util_unbounded: bool, whale_borrow_eth: f64) -> Seed {
    let mut b = LendingTest::new()
        .standard_two_asset()
        .with_min_borrow_collateral_disabled();
    if max_util_unbounded {
        b = b.with_max_utilization_disabled_all_markets();
    }
    let mut t = b.build();
    let runner = t.deploy_script_runner();
    t.fund_runner(&runner, "ETH", 50 * U);
    let runner_account = t
        .run_script(&runner, &vec![&t.env, supply_op(&t, 0, "ETH", 50 * U)])
        .expect("runner supplies ETH");
    t.supply(BOB, "ETH", 50.0);
    // ALICE's tiny USDC collateral backs ETH debt; a USDC crash makes the account dust-insolvent.
    t.supply(ALICE, "USDC", 10.0);
    t.borrow(ALICE, "ETH", 0.003);
    if whale_borrow_eth > 0.0 {
        t.supply(CAROL, "USDC", 200_000.0);
        t.borrow(CAROL, "ETH", whale_borrow_eth);
    }
    t.set_price("USDC", usd_cents(10));
    let victim = t.account_id(ALICE);
    Seed {
        t,
        runner,
        runner_account,
        victim,
    }
}

#[test]
fn exit_then_clean_then_re_enter_dodges_the_write_down_when_utilization_allows() {
    let Seed {
        t,
        runner,
        runner_account,
        victim,
    } = seed(true, 0.0);
    let bob_before = t.supply_balance_raw(BOB, "ETH");
    let ops: Vec<Op> = vec![
        &t.env,
        withdraw_op(&t, runner_account, "ETH", 0, None),
        Op::CleanBadDebt(AccountOp { account_id: victim }),
        supply_op(&t, 0, "ETH", 50 * U),
    ];
    let new_id = t
        .run_script(&runner, &ops)
        .expect("atomic exit, clean, re-enter");
    assert!(new_id > 0 && new_id != runner_account);
    assert!(
        t.supply_balance_raw_for(new_id, "ETH") >= 50 * U - 1,
        "the runner kept its whole stake"
    );
    assert!(
        t.supply_balance_raw(BOB, "ETH") < bob_before,
        "BOB absorbed the entire write-down"
    );
}

#[test]
fn the_exit_is_capped_by_max_utilization_and_the_script_reverts_whole() {
    // 48 of 100 ETH borrowed: cash covers the runner's 50, but the exit
    // would leave 48 of 50 in use, past the 95 percent ceiling.
    let Seed {
        t,
        runner,
        runner_account,
        victim,
    } = seed(false, 48.0);
    let bob_before = t.supply_balance_raw(BOB, "ETH");
    let ops: Vec<Op> = vec![
        &t.env,
        withdraw_op(&t, runner_account, "ETH", 0, None),
        Op::CleanBadDebt(AccountOp { account_id: victim }),
    ];
    assert_contract_error(
        t.run_script(&runner, &ops).map(|_| ()),
        errors::UTILIZATION_ABOVE_MAX,
    );
    assert_eq!(
        t.supply_balance_raw(BOB, "ETH"),
        bob_before,
        "nothing committed"
    );
}
