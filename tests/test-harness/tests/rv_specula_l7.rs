//! Second opinion on Specula lending CR-10 and CR-11: the 1.05 restamp gate
//! in a multi-leg withdraw or supply reads each leg against the legs that
//! have not been processed yet.
//!
//! CR-10: `merge_withdraw_leg` runs `refresh_supply_risk_params` per leg, and
//! `clears_min_hf` values the account with this leg at its new balance and
//! every later leg at its pre-withdraw balance. A liquidator-favoring tuple
//! can therefore be stamped on leg 1 while leg 2 takes the final health factor
//! below 1.05 (but not below 1, which `require_post_pool_risk_gates` still
//! enforces). The same final state is reachable with two single-leg
//! withdrawals, so the gate was never a post-condition of a transaction.
//!
//! CR-11: `process_deposit` does the same in the other direction. A small
//! first leg is checked before a large second leg lands, keeps the stale
//! tuple, and the leg order decides the outcome. `update_account_threshold`
//! with `has_risks` applies the listing afterwards.
//!
//! Prices: USDC 1, ETH 2000, WBTC 60000. Stored default tuple is
//! (8000, 500, 1200); `edit_asset_in_spoke` always writes fees 0.

use test_harness::{hub_asset, LendingTest, ALICE, BOB, HARNESS_SPOKE};

const WAD: i128 = 1_000_000_000_000_000_000;
const GATE: i128 = 1_050_000_000_000_000_000;

const STALE_TUPLE: (u32, u32, u32) = (8_000, 500, 1_200);
/// USDC after `edit_asset_in_spoke(.., 6_900 | 6_500, 7_000, 600)`.
const TIGHT_TUPLE: (u32, u32, u32) = (7_000, 600, 0);

/// `(threshold, bonus, fees)` stored on the account's `asset` leg.
fn stamped_tuple(t: &LendingTest, account: u64, asset: &str) -> (u32, u32, u32) {
    let (supplies, _) = t.ctrl_client().get_account_positions(&account);
    let leg = supplies.get(hub_asset(t.resolve_asset(asset))).unwrap();
    (
        leg.liquidation_threshold,
        leg.liquidation_bonus,
        leg.liquidation_fees,
    )
}

/// Alice: 10_000 USDC + 5 ETH (20_000 USD) against 0.2 WBTC (12_000 USD).
/// USDC is then relisted at LTV 6_900 / LT 7_000 / bonus 600, which favors
/// the liquidator on every field. ETH keeps LTV 7_500 / LT 8_000.
fn two_leg_withdraw_account() -> (LendingTest, u64) {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
    t.supply(BOB, "WBTC", 1.0);
    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 5.0);
    t.borrow(ALICE, "WBTC", 0.2);
    let account = t.account_id(ALICE);
    assert_eq!(stamped_tuple(&t, account, "USDC"), STALE_TUPLE);
    t.edit_asset_in_spoke("USDC", HARNESS_SPOKE, true, true, 6_900, 7_000, 600);
    (t, account)
}

/// Final state of both withdraw tests: USDC 7_000, ETH 4.8 (9_600 USD).
/// HF at the new USDC threshold = (0.7 * 7_000 + 0.8 * 9_600) / 12_000 =
/// 1.0483; LTV cover = 0.69 * 7_000 + 0.75 * 9_600 = 12_030 >= 12_000.
fn assert_final_state_below_gate(t: &LendingTest, account: u64) {
    let hf = t.health_factor_for_raw(ALICE, account);
    assert!(hf >= WAD, "post-pool solvency holds: {hf}");
    assert!(hf < GATE, "final HF sits under the 1.05 gate: {hf}");
    assert_eq!(
        stamped_tuple(t, account, "USDC"),
        TIGHT_TUPLE,
        "the liquidator-favoring tuple is stamped at a final HF below 1.05"
    );
    assert_eq!(stamped_tuple(t, account, "ETH"), STALE_TUPLE);
}

#[test]
fn cr10_multi_leg_withdraw_stamps_leg_one_against_leg_two_pre_withdraw_balance() {
    let (mut t, account) = two_leg_withdraw_account();
    // USDC leg gate: (0.7 * 7_000 + 0.8 * 10_000) / 12_000 = 1.075 >= 1.05.
    // The ETH leg then removes 400 USD and the final HF is 1.0483.
    t.withdraw_bulk(ALICE, &[("USDC", 3_000.0), ("ETH", 0.2)]);
    assert_final_state_below_gate(&t, account);
    t.assert_spoke_usage_matches_positions();
}

#[test]
fn cr10_two_single_leg_withdrawals_reach_the_same_state() {
    let (mut t, account) = two_leg_withdraw_account();
    t.withdraw(ALICE, "USDC", 3_000.0);
    assert_eq!(stamped_tuple(&t, account, "USDC"), TIGHT_TUPLE);
    t.withdraw(ALICE, "ETH", 0.2);
    assert_final_state_below_gate(&t, account);
}

#[test]
fn cr10_reversed_leg_order_checks_the_final_state_and_keeps_the_stale_tuple() {
    let (mut t, account) = two_leg_withdraw_account();
    // ETH first (no tuple change), then USDC: its gate now reads the final
    // state, 1.0483 < 1.05, and holds the stale tuple.
    t.withdraw_bulk(ALICE, &[("ETH", 0.2), ("USDC", 3_000.0)]);
    assert_eq!(stamped_tuple(&t, account, "USDC"), STALE_TUPLE);
    // Reported HF uses the retained LT 8_000: 0.8 * 16_600 / 12_000 = 1.1067.
    let hf = t.health_factor_for_raw(ALICE, account);
    assert_eq!(hf, 1_106_666_666_666_666_666, "{hf}");
}

/// Alice: 10_000 USDC against 3.5 ETH (7_000 USD). USDC relisted at
/// LTV 6_500 / LT 7_000 / bonus 600: HF at the new threshold is 1.0.
fn stale_supply_account() -> (LendingTest, u64) {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.5);
    let account = t.account_id(ALICE);
    assert_eq!(stamped_tuple(&t, account, "USDC"), STALE_TUPLE);
    t.edit_asset_in_spoke("USDC", HARNESS_SPOKE, true, true, 6_500, 7_000, 600);
    (t, account)
}

#[test]
fn cr11_multi_leg_supply_order_decides_whether_the_stale_tuple_survives() {
    // USDC first: gate reads 0.7 * 10_100 / 7_000 = 1.01 < 1.05 before the
    // 6_000 USD of WBTC lands, so the stale tuple stays.
    let (mut t, account) = stale_supply_account();
    t.supply_bulk(ALICE, &[("USDC", 100.0), ("WBTC", 0.1)]);
    assert_eq!(stamped_tuple(&t, account, "USDC"), STALE_TUPLE);

    // WBTC first: the USDC gate reads (0.7 * 10_100 + 0.8 * 6_000) / 7_000 =
    // 1.70 and applies the listing.
    let (mut t2, account2) = stale_supply_account();
    t2.supply_bulk(ALICE, &[("WBTC", 0.1), ("USDC", 100.0)]);
    assert_eq!(stamped_tuple(&t2, account2, "USDC"), TIGHT_TUPLE);

    // The permissionless refresh repairs the first ordering.
    t.update_account_threshold(true, &[account]);
    assert_eq!(stamped_tuple(&t, account, "USDC"), TIGHT_TUPLE);
    assert!(t.health_factor_for_raw(ALICE, account) >= GATE);
}
