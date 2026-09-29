//! GH-22. A supply to a leg re-runs the risk restamp. It cannot tighten a
//! threshold on an account whose health factor would sit below 1.05
//! afterwards, and it does apply a loosened one. The gate reads the health
//! factor after the deposit, so a deposit counts toward its own gate.

use controller::types::PositionMode;
use test_harness::{
    build_aggregator_swap, f64_to_i128, hub_asset, LendingTest, ALICE, BOB, CAROL, HARNESS_SPOKE,
};

fn stamped_threshold(t: &LendingTest, account: u64) -> u32 {
    stamped_tuple(t, account).0
}

/// `(threshold, bonus, fees)` stored on the account's USDC leg.
fn stamped_tuple(t: &LendingTest, account: u64) -> (u32, u32, u32) {
    let (supplies, _) = t.ctrl_client().get_account_positions(&account);
    let leg = supplies.get(hub_asset(t.resolve_asset("USDC"))).unwrap();
    (
        leg.liquidation_threshold,
        leg.liquidation_bonus,
        leg.liquidation_fees,
    )
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
