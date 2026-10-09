//! A borrow, a withdrawal or a strategy restamps the stored LTV of every
//! listed supply leg. Each changed leg is reported as a `ParamUpd` delta in
//! the same position batch.

use crate::shared::{as_vec, data_for_topic};
use soroban_sdk::testutils::Events;
use soroban_sdk::xdr::ScVal;
use soroban_sdk::TryFromVal;
use test_harness::{
    build_aggregator_swap, f64_to_i128, hub_asset, LendingTest, ALICE, BOB, HARNESS_SPOKE,
};

const WITHDRAW: u32 = 2;
const PARAM_UPD: u32 = 7;
const RP_COL_WD: u32 = 10;

/// `(action, asset, ltv, amount)` of each deposit leg in the last batch.
fn deposit_legs(t: &LendingTest) -> std::vec::Vec<(u32, ScVal, u32, i128)> {
    let events = t.env.events().all();
    let batches = data_for_topic(&events, "position", "batch_update");
    let batch = as_vec(batches.last().expect("a position batch"));
    as_vec(&batch[2])
        .iter()
        .map(|leg| {
            let leg = as_vec(leg);
            let u32_at = |i: usize| match &leg[i] {
                ScVal::U32(v) => *v,
                other => panic!("leg field {i} is not u32: {other:?}"),
            };
            let amount = match &leg[5] {
                ScVal::I128(v) => i128::from(v),
                other => panic!("leg amount is not i128: {other:?}"),
            };
            (u32_at(0), leg[2].clone(), u32_at(8), amount)
        })
        .collect()
}

fn asset_val(t: &LendingTest, name: &str) -> ScVal {
    ScVal::try_from_val(&t.env, &t.resolve_asset(name).to_val()).unwrap()
}

fn stored_ltv(t: &LendingTest, account: u64, name: &str) -> u32 {
    let (supplies, _) = t.ctrl_client().get_account_positions(&account);
    supplies
        .get(hub_asset(t.resolve_asset(name)))
        .unwrap()
        .loan_to_value
}

/// USDC and WBTC collateral against ETH debt, then WBTC relisted at LTV 6_000.
fn account_with_stale_wbtc_ltv() -> (LendingTest, u64) {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(ALICE, "WBTC", 0.1);
    t.borrow(ALICE, "ETH", 0.5);
    let account = t.account_id(ALICE);
    t.edit_asset_in_spoke("WBTC", HARNESS_SPOKE, true, true, 6_000, 8_000, 500);
    assert_eq!(stored_ltv(&t, account, "WBTC"), 7_500);
    (t, account)
}

#[test]
fn a_borrow_reports_the_restamp_of_a_leg_it_does_not_move() {
    let (mut t, account) = account_with_stale_wbtc_ltv();
    t.borrow(ALICE, "ETH", 0.1);
    let legs = deposit_legs(&t);
    assert_eq!(stored_ltv(&t, account, "WBTC"), 6_000);
    assert_eq!(
        legs,
        std::vec![(PARAM_UPD, asset_val(&t, "WBTC"), 6_000, 0)]
    );
}

#[test]
fn a_withdraw_reports_the_restamp_after_its_own_leg() {
    let (mut t, account) = account_with_stale_wbtc_ltv();
    t.withdraw(ALICE, "USDC", 100.0);
    let legs = deposit_legs(&t);
    assert_eq!(stored_ltv(&t, account, "WBTC"), 6_000);
    assert_eq!(legs.len(), 2);
    assert_eq!(
        (legs[0].0, legs[0].1.clone()),
        (WITHDRAW, asset_val(&t, "USDC"))
    );
    assert_eq!(legs[1], (PARAM_UPD, asset_val(&t, "WBTC"), 6_000, 0));
}

#[test]
fn a_strategy_reports_the_restamp_of_a_leg_it_does_not_move() {
    let (mut t, account) = account_with_stale_wbtc_ltv();
    t.fund_router("ETH", 10.0);
    let steps = build_aggregator_swap(&t, "USDC", "ETH", 0, f64_to_i128(0.3, 7));
    t.repay_debt_with_collateral(ALICE, "USDC", 600.0, "ETH", &steps, false);
    let legs = deposit_legs(&t);
    assert_eq!(stored_ltv(&t, account, "WBTC"), 6_000);
    assert_eq!(legs.len(), 2);
    assert_eq!(
        (legs[0].0, legs[0].1.clone()),
        (RP_COL_WD, asset_val(&t, "USDC"))
    );
    assert_eq!(legs[1], (PARAM_UPD, asset_val(&t, "WBTC"), 6_000, 0));
}

#[test]
fn a_leg_the_flow_moves_is_reported_once_with_its_new_ltv() {
    let (mut t, account) = account_with_stale_wbtc_ltv();
    t.withdraw(ALICE, "WBTC", 0.01);
    let legs = deposit_legs(&t);
    assert_eq!(stored_ltv(&t, account, "WBTC"), 6_000);
    assert_eq!(legs.len(), 1);
    assert_eq!(
        (legs[0].0, legs[0].1.clone(), legs[0].2),
        (WITHDRAW, asset_val(&t, "WBTC"), 6_000)
    );
}

#[test]
fn a_borrow_without_a_stale_ltv_reports_no_param_update() {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(ALICE, "WBTC", 0.1);
    t.borrow(ALICE, "ETH", 0.5);
    t.borrow(ALICE, "ETH", 0.1);
    assert!(deposit_legs(&t).is_empty());
}
