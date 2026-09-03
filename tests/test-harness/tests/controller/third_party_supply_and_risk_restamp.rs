//! GH-22. A stranger's one-stroop top-up re-runs the risk restamp. It cannot
//! tighten a threshold on an account whose health factor would sit below
//! 1.05 afterwards, and it does apply a loosened one.

use test_harness::{hub_asset, LendingTest, ALICE, BOB, CAROL, HARNESS_SPOKE};

fn stamped_threshold(t: &LendingTest, account: u64) -> u32 {
    let (supplies, _) = t.ctrl_client().get_account_positions(&account);
    supplies
        .get(hub_asset(t.resolve_asset("USDC")))
        .unwrap()
        .liquidation_threshold
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
