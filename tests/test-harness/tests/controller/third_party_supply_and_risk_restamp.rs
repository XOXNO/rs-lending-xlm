//! GH-22. A supply to a leg re-runs the risk restamp. It cannot tighten a
//! threshold on an account whose health factor would sit below 1.05
//! afterwards, and it does apply a loosened one. The gate reads the health
//! factor after the deposit, so a deposit counts toward its own gate.

use crate::shared::{as_vec, data_for_topic};
use controller::types::PositionMode;
use soroban_sdk::{testutils::Events, xdr::ScVal, TryFromVal, Vec};
use test_harness::{
    build_aggregator_swap, f64_to_i128, hub_asset, LendingTest, ALICE, BOB, CAROL, HARNESS_SPOKE,
};

fn stamped_threshold(t: &LendingTest, account: u64) -> u32 {
    stamped_tuple(t, account).0
}

/// `(threshold, bonus, fees)` stored on the account's USDC leg.
fn stamped_tuple(t: &LendingTest, account: u64) -> (u32, u32, u32) {
    asset_tuple(t, account, "USDC")
}

fn asset_tuple(t: &LendingTest, account: u64, asset: &str) -> (u32, u32, u32) {
    let (supplies, _) = t.ctrl_client().get_account_positions(&account);
    let leg = supplies.get(hub_asset(t.resolve_asset(asset))).unwrap();
    (
        leg.liquidation_threshold,
        leg.liquidation_bonus,
        leg.liquidation_fees,
    )
}

fn assert_batch_events(t: &LendingTest, account: u64, assets: &[&str]) {
    let batches = data_for_topic(&t.env.events().all(), "position", "batch_update");
    let batch = as_vec(batches.last().unwrap());
    let legs = as_vec(&batch[2]);
    let (supplies, _) = t.ctrl_client().get_account_positions(&account);
    assert_eq!(legs.len(), assets.len());
    for (leg, asset) in legs.iter().zip(assets) {
        let leg = as_vec(leg);
        let address = t.resolve_asset(asset);
        assert_eq!(
            leg[2],
            ScVal::try_from_val(&t.env, &address.to_val()).unwrap()
        );
        let position = supplies.get(hub_asset(address)).unwrap();
        for (field, value) in [
            (6, position.liquidation_threshold),
            (7, position.liquidation_bonus),
            (8, position.loan_to_value),
            (9, position.liquidation_fees),
        ] {
            assert_eq!(leg[field], ScVal::U32(value));
        }
    }
    t.assert_spoke_usage_matches_positions();
}

const STALE_TUPLE: (u32, u32, u32) = (8_000, 500, 1_200);
const TIGHT_TUPLE: (u32, u32, u32) = (7_000, 600, 0);

/// 10_000 USDC against 7_000 USD of ETH, then relisted at `TIGHT_TUPLE`:
/// health factor at the new threshold 0.7 * 10_000 / 7_000 = 1.0.
fn stale_account() -> (LendingTest, u64) {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.5);
    let account = t.account_id(ALICE);
    assert_eq!(stamped_tuple(&t, account), STALE_TUPLE);
    t.edit_asset_in_spoke("USDC", HARNESS_SPOKE, true, true, 6_500, 7_000, 600);
    (t, account)
}

#[test]
fn a_stranger_cannot_tighten_a_stale_threshold_below_the_update_floor() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.5);
    let account = t.account_id(ALICE);
    assert_eq!(stamped_threshold(&t, account), 8_000);
    // Health factor with the old threshold: 0.8 * 10_000 / 7_000 = 1.14.
    // Governance tightens to 70 percent: the hypothetical HF would be 1.0, below 1.05.
    t.edit_asset_in_spoke("USDC", HARNESS_SPOKE, true, true, 6_500, 7_000, 500);
    t.supply_to(CAROL, account, "USDC", 0.0000001);
    assert_eq!(
        stamped_threshold(&t, account),
        8_000,
        "the tightening is gated on the update floor"
    );
}

#[test]
fn a_stranger_applies_a_loosened_threshold_immediately() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    let account = t.account_id(ALICE);
    t.edit_asset_in_spoke("USDC", HARNESS_SPOKE, true, true, 7_500, 8_500, 500);
    t.supply_to(CAROL, account, "USDC", 0.0000001);
    assert_eq!(
        stamped_threshold(&t, account),
        8_500,
        "loosening never needs a gate"
    );
}

#[test]
fn a_stranger_can_tighten_when_the_account_clears_the_update_floor() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    let account = t.account_id(ALICE);
    t.edit_asset_in_spoke("USDC", HARNESS_SPOKE, true, true, 6_500, 7_000, 500);
    t.supply_to(CAROL, account, "USDC", 0.0000001);
    assert_eq!(
        stamped_threshold(&t, account),
        7_000,
        "HF stays far above 1.05, so the stamp moves"
    );
}

#[test]
fn an_owner_deposit_that_lifts_the_account_over_the_floor_applies_the_whole_tuple() {
    let (mut t, account) = stale_account();
    // After the deposit: 0.7 * 20_000 / 7_000 = 2.0.
    t.supply(ALICE, "USDC", 10_000.0);
    assert_eq!(
        stamped_tuple(&t, account),
        TIGHT_TUPLE,
        "the deposit counts toward its own gate"
    );
    t.assert_spoke_usage_matches_positions();
}

#[test]
fn a_deposit_that_ends_just_under_the_floor_keeps_the_stale_tuple() {
    // 0.7 * 10_499 / 7_000 = 1.0499.
    let (mut t, account) = stale_account();
    t.supply(ALICE, "USDC", 499.0);
    assert_eq!(stamped_tuple(&t, account), STALE_TUPLE);
}

#[test]
fn a_deposit_that_ends_just_over_the_floor_applies_the_tuple() {
    // 0.7 * 10_501 / 7_000 = 1.0501.
    let (mut t, account) = stale_account();
    t.supply(ALICE, "USDC", 501.0);
    assert_eq!(stamped_tuple(&t, account), TIGHT_TUPLE);
}

#[test]
fn a_stranger_donation_that_lifts_the_account_over_the_floor_applies_the_tuple() {
    // 0.7 * 10_600 / 7_000 = 1.06.
    let (mut t, account) = stale_account();
    t.supply_to(CAROL, account, "USDC", 600.0);
    assert_eq!(stamped_tuple(&t, account), TIGHT_TUPLE);
    t.assert_spoke_usage_matches_positions();
}

#[test]
fn a_withdraw_that_ends_under_the_floor_keeps_the_stale_tuple() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10_600.0);
    t.borrow(ALICE, "ETH", 3.5);
    let account = t.account_id(ALICE);
    t.edit_asset_in_spoke("USDC", HARNESS_SPOKE, true, true, 6_900, 7_000, 600);
    // 0.7 * 10_600 / 7_000 = 1.06 before, 0.7 * 10_400 / 7_000 = 1.04 after.
    t.withdraw(ALICE, "USDC", 200.0);
    assert_eq!(stamped_tuple(&t, account), STALE_TUPLE);
    t.assert_spoke_usage_matches_positions();
}

#[test]
fn supply_batch_refresh_sees_all_deposits_in_either_order() {
    for order in [
        [("USDC", 1.0), ("ETH", 0.25)],
        [("ETH", 0.25), ("USDC", 1.0)],
    ] {
        let (mut t, account) = stale_account();
        t.supply_bulk(ALICE, &order);
        assert_batch_events(&t, account, &order.map(|(asset, _)| asset));
        assert_eq!(stamped_tuple(&t, account), TIGHT_TUPLE);
        t.assert_supply_near(ALICE, "USDC", 10_001.0, 0.000001);
        t.assert_supply_near(ALICE, "ETH", 0.25, 0.000001);
    }
}

#[test]
fn withdraw_batch_refresh_sees_all_exits_in_either_order() {
    for order in [
        [("USDC", 300.0), ("ETH", 0.2)],
        [("ETH", 0.2), ("USDC", 300.0)],
    ] {
        let mut t = LendingTest::new().standard_two_asset().build();
        t.supply(BOB, "ETH", 100.0);
        t.supply_bulk(ALICE, &[("USDC", 10_600.0), ("ETH", 0.25)]);
        t.borrow(ALICE, "ETH", 3.5);
        let account = t.account_id(ALICE);
        t.edit_asset_in_spoke("USDC", HARNESS_SPOKE, true, true, 6_900, 7_000, 600);
        let mut withdrawals = Vec::new(&t.env);
        for (asset, amount) in order {
            withdrawals.push_back((hub_asset(t.resolve_asset(asset)), f64_to_i128(amount, 7)));
        }
        let caller = t.users.get(ALICE).unwrap().address.clone();
        let paid = t
            .ctrl_client()
            .withdraw(&caller, &account, &withdrawals, &None);
        assert_eq!(paid, withdrawals);
        assert_batch_events(&t, account, &order.map(|(asset, _)| asset));
        assert_eq!(stamped_tuple(&t, account), STALE_TUPLE);
        t.assert_supply_near(ALICE, "USDC", 10_300.0, 0.000001);
        t.assert_supply_near(ALICE, "ETH", 0.05, 0.000001);
    }
}

#[test]
fn multiple_pending_tightenings_use_asset_key_order() {
    let mut expected = None;
    for order in [
        [("USDC", 1.0), ("ETH", 0.0005)],
        [("ETH", 0.0005), ("USDC", 1.0)],
    ] {
        let mut t = LendingTest::new().standard_two_asset().build();
        t.supply(BOB, "ETH", 100.0);
        t.supply_bulk(ALICE, &[("USDC", 5_999.0), ("ETH", 2.9995)]);
        t.borrow(ALICE, "ETH", 4.0);
        let account = t.account_id(ALICE);
        for asset in ["USDC", "ETH"] {
            t.edit_asset_in_spoke(asset, HARNESS_SPOKE, true, true, 6_000, 6_500, 600);
        }
        t.supply_bulk(ALICE, &order);
        assert_batch_events(&t, account, &order.map(|(asset, _)| asset));
        let tuples = (
            asset_tuple(&t, account, "USDC"),
            asset_tuple(&t, account, "ETH"),
        );
        let (supplies, _) = t.ctrl_client().get_account_positions(&account);
        let thresholds: std::vec::Vec<_> = supplies
            .iter()
            .map(|(_, position)| position.liquidation_threshold)
            .collect();
        assert_eq!(thresholds, [6_500, 8_000]);
        if let Some(previous) = expected {
            assert_eq!(tuples, previous);
        }
        expected = Some(tuples);
    }
}

#[test]
fn a_multiply_leg_counts_its_own_collateral_toward_the_floor() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(BOB, "ETH", 100.0);
    t.fund_router("USDC", 100_000.0);
    let alice = t.get_or_create_user(ALICE);
    let usdc = hub_asset(t.resolve_asset("USDC"));
    let eth = hub_asset(t.resolve_asset("ETH"));
    let usdc_raw = |amount: f64| f64_to_i128(amount, 7);
    t.resolve_market("USDC")
        .token_admin
        .mint(&alice, &usdc_raw(10_000.0));
    let swap = build_aggregator_swap(&t, "ETH", "USDC", 0, usdc_raw(6_990.0));

    let account = t.ctrl_client().multiply(
        &alice,
        &0,
        &HARNESS_SPOKE,
        &usdc,
        &f64_to_i128(3.5, 7),
        &eth,
        &PositionMode::Multiply,
        &swap,
        &Some((usdc.clone(), usdc_raw(10_000.0))),
        &None,
    );
    assert_eq!(stamped_tuple(&t, account), STALE_TUPLE);
    t.edit_asset_in_spoke("USDC", HARNESS_SPOKE, true, true, 6_500, 7_000, 600);

    // At the new threshold: 0.7 * 16_990 / 14_000 = 0.85 between borrow and
    // deposit, 0.7 * 23_980 / 14_000 = 1.2 after.
    t.ctrl_client().multiply(
        &alice,
        &account,
        &HARNESS_SPOKE,
        &usdc,
        &f64_to_i128(3.5, 7),
        &eth,
        &PositionMode::Multiply,
        &swap,
        &None,
        &None,
    );
    assert_eq!(
        stamped_tuple(&t, account),
        TIGHT_TUPLE,
        "the leg's own collateral counts toward the gate"
    );
    t.assert_spoke_usage_matches_positions();
}

/// 10_000 USDC against `eth_debt` ETH and optional USDC debt on the three-asset
/// book, then relisted at `TIGHT_TUPLE` with LTV `ltv`.
fn stale_strategy_account(eth_debt: f64, usdc_debt: f64, ltv: u32) -> (LendingTest, u64) {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10_000.0);
    if usdc_debt > 0.0 {
        t.borrow(ALICE, "USDC", usdc_debt);
    }
    t.borrow(ALICE, "ETH", eth_debt);
    let account = t.account_id(ALICE);
    assert_eq!(stamped_tuple(&t, account), STALE_TUPLE);
    t.edit_asset_in_spoke("USDC", HARNESS_SPOKE, true, true, ltv, 7_000, 600);
    (t, account)
}

/// The last USDC delta of the strategy's batch carries the stored tuple.
fn assert_last_usdc_delta_matches(t: &LendingTest, account: u64) {
    let batches = data_for_topic(&t.env.events().all(), "position", "batch_update");
    let batch = as_vec(batches.last().unwrap());
    let usdc = t.resolve_asset("USDC");
    let usdc_val = ScVal::try_from_val(&t.env, &usdc.to_val()).unwrap();
    let leg = as_vec(&batch[2])
        .iter()
        .rev()
        .find(|leg| as_vec(leg)[2] == usdc_val)
        .unwrap();
    let leg = as_vec(leg);
    let (threshold, bonus, fees) = stamped_tuple(t, account);
    assert_eq!(
        (leg[6].clone(), leg[7].clone(), leg[9].clone()),
        (ScVal::U32(threshold), ScVal::U32(bonus), ScVal::U32(fees))
    );
    t.assert_spoke_usage_matches_positions();
}

/// CR-11. At the new threshold: 0.7 * 5_000 / 6_000 = 0.58 between the USDC
/// withdrawal and the WBTC deposit, (3_500 + 0.8 * 5_000) / 6_000 = 1.25 after.
#[test]
fn a_swap_collateral_refresh_reads_the_finished_swap() {
    let (mut t, account) = stale_strategy_account(3.0, 0.0, 6_500);
    t.fund_router("WBTC", 10.0);
    let swap = build_aggregator_swap(&t, "USDC", "WBTC", 0, f64_to_i128(0.0833333, 7));

    t.swap_collateral(ALICE, "USDC", 5_000.0, "WBTC", &swap);

    assert_last_usdc_delta_matches(&t, account);
    assert_eq!(stamped_tuple(&t, account), TIGHT_TUPLE);
}

/// CR-11. At the new threshold: 0.7 * 7_000 / 7_000 = 0.7 between the USDC
/// withdrawal and the ETH repayment, 0.7 * 7_000 / 4_000 = 1.225 after.
#[test]
fn a_swap_repayment_refresh_reads_the_repaid_debt() {
    let (mut t, account) = stale_strategy_account(3.5, 0.0, 6_500);
    t.fund_router("ETH", 10.0);
    let swap = build_aggregator_swap(&t, "USDC", "ETH", 0, f64_to_i128(1.5, 7));

    t.repay_debt_with_collateral(ALICE, "USDC", 3_000.0, "ETH", &swap, false);

    assert_last_usdc_delta_matches(&t, account);
    assert_eq!(stamped_tuple(&t, account), TIGHT_TUPLE);
}

/// CR-11. A same-market net settle of 3_000 USDC: the gate reads 0.7 between
/// the supply leg and the debt leg, 0.7 * 7_000 / 4_000 = 1.225 after.
#[test]
fn a_net_settle_refresh_reads_both_settled_legs() {
    let (mut t, account) = stale_strategy_account(1.75, 3_500.0, 6_500);
    let no_route = soroban_sdk::Bytes::new(&t.env);

    t.repay_debt_with_collateral(ALICE, "USDC", 3_000.0, "USDC", &no_route, false);

    assert_last_usdc_delta_matches(&t, account);
    assert_eq!(stamped_tuple(&t, account), TIGHT_TUPLE);
}

/// CR-11. The finalization gate still holds: with LTV 6_900, a swap that ends
/// at (6_300 + 0.8 * 1_000) / 6_800 = 1.044 keeps the stale tuple.
#[test]
fn a_strategy_that_ends_under_the_floor_keeps_the_stale_tuple() {
    let (mut t, account) = stale_strategy_account(3.4, 0.0, 6_900);
    t.fund_router("WBTC", 10.0);
    let swap = build_aggregator_swap(&t, "USDC", "WBTC", 0, f64_to_i128(0.0166666, 7));

    t.swap_collateral(ALICE, "USDC", 1_000.0, "WBTC", &swap);

    assert_last_usdc_delta_matches(&t, account);
    assert_eq!(stamped_tuple(&t, account), STALE_TUPLE);
}
