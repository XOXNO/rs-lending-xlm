//! GH-10. Fifty repetitions of a verb cycle inside one invocation extract
//! nothing: the runner's wallet never rises, spoke usage tracks positions,
//! and the pool stays backed.

use crate::helpers::{borrow_op, repay_op, supply_op, withdraw_op, Snapshot};
use script_runner::{Op, LAST_CREATED};
use soroban_sdk::{vec, Vec};
use test_harness::{LendingTest, BOB};

const U: i128 = 10_000_000;

fn setup() -> (LendingTest, soroban_sdk::Address) {
    let mut t = LendingTest::new().standard_two_asset_dust_disabled();
    t.supply(BOB, "ETH", 100.0);
    t.supply(BOB, "USDC", 50_000.0);
    let runner = t.deploy_script_runner();
    t.fund_runner(&runner, "USDC", 10_000 * U);
    t.fund_runner(&runner, "ETH", U);
    (t, runner)
}

#[test]
fn fifty_supply_borrow_repay_withdraw_cycles_leave_the_runner_no_richer() {
    let (t, runner) = setup();
    let usdc_start = t.runner_wallet(&runner, "USDC");
    let eth_start = t.runner_wallet(&runner, "ETH");
    let mut ops: Vec<Op> = vec![&t.env, supply_op(&t, 0, "USDC", 1_000 * U)];
    for i in 0..50i128 {
        ops.push_back(supply_op(&t, LAST_CREATED, "USDC", 100 * U + i));
        ops.push_back(borrow_op(&t, LAST_CREATED, "ETH", U / 100 + i, None));
        ops.push_back(repay_op(&t, LAST_CREATED, "ETH", U / 100 + i + 1));
        ops.push_back(withdraw_op(&t, LAST_CREATED, "USDC", 100 * U + i, None));
    }
    ops.push_back(withdraw_op(&t, LAST_CREATED, "USDC", 0, None));
    t.run_script(&runner, &ops)
        .expect("the cycle is legal fifty times over");
    let usdc_end = t.runner_wallet(&runner, "USDC");
    let eth_end = t.runner_wallet(&runner, "ETH");
    assert!(usdc_end <= usdc_start && usdc_start - usdc_end <= 100);
    assert!(eth_end <= eth_start && eth_start - eth_end <= 100);
    let s = Snapshot::take(&t, &runner, &["USDC", "ETH"], &[]);
    for (asset, supplied, borrowed, revenue, _, _, _) in &s.markets {
        let (_, used_supply, used_borrow) = s.usage.iter().find(|u| &u.0 == asset).unwrap();
        assert_eq!(
            *used_supply,
            supplied - revenue,
            "{asset}: spoke supply usage must equal the pool's supplied shares less revenue"
        );
        assert_eq!(
            used_borrow, borrowed,
            "{asset}: spoke borrow usage must equal the pool's debt shares"
        );
    }
}

#[test]
fn fifty_supply_then_withdraw_all_cycles_return_at_most_the_deposit() {
    let (t, runner) = setup();
    let start = t.runner_wallet(&runner, "USDC");
    let mut ops: Vec<Op> = Vec::new(&t.env);
    for i in 0..50i128 {
        ops.push_back(supply_op(&t, 0, "USDC", 777 * U + i));
        ops.push_back(withdraw_op(&t, LAST_CREATED, "USDC", 0, None));
    }
    t.run_script(&runner, &ops)
        .expect("open and close fifty accounts");
    let end = t.runner_wallet(&runner, "USDC");
    assert!(end <= start && start - end <= 50);
}
