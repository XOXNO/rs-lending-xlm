//! A borrow, a withdrawal or a strategy restamps the stored LTV of every
//! listed supply leg. Each changed leg is reported as a `ParamUpd` delta in
//! the same position batch.

use crate::shared::{as_vec, data_for_topic};
use controller::constants::WAD;
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

fn stored_wbtc_tuple(t: &LendingTest, account_id: u64) -> (u32, u32, u32, u32) {
    let (supplies, _) = t.ctrl_client().get_account_positions(&account_id);
    let p = supplies.get(hub_asset(t.resolve_asset("WBTC"))).unwrap();
    (
        p.loan_to_value,
        p.liquidation_threshold,
        p.liquidation_bonus,
        p.liquidation_fees,
    )
}

#[test]
fn a_loosened_listing_restamps_ltv_and_threshold_together_on_borrow() {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
    t.supply(BOB, "USDC", 100_000.0);
    t.supply(ALICE, "WBTC", 0.5);
    let account = t.account_id(ALICE);
    t.edit_asset_config("WBTC", |c| {
        c.loan_to_value = 8_000;
        c.liquidation_threshold = 8_500;
        c.liquidation_bonus = 400;
    });

    assert_eq!(
        t.ctrl_client().get_ltv_collateral_usd(&account),
        24_000 * WAD
    );
    t.borrow(ALICE, "USDC", 24_000.0);

    assert_eq!(stored_wbtc_tuple(&t, account), (8_000, 8_500, 400, 1_200));
    assert_eq!(t.health_factor_raw(ALICE), 1_062_500_000_000_000_000);
    t.advance_time(5);
    assert!(!t.can_be_liquidated(ALICE));
}

fn list_wbtc(t: &LendingTest, (ltv, threshold, bonus, fees): (u32, u32, u32, u32)) {
    t.edit_asset_config("WBTC", |c| {
        c.loan_to_value = ltv;
        c.liquidation_threshold = threshold;
        c.liquidation_bonus = bonus;
        c.liquidation_fees = fees;
    });
}

/// USD WAD per raw unit of USDC (7 decimals, priced at 1 USD).
const WAD_PER_USDC_UNIT: i128 = WAD / 10_000_000;

/// What an account ends with after it borrows all the USDC the view allows.
#[derive(Debug, PartialEq)]
struct AtTheReportedLimit {
    /// `get_ltv_collateral_usd` before the borrow, in USD WAD.
    limit_wad: i128,
    /// Stored WBTC `(ltv, threshold, bonus, fees)` after the borrow, in BPS.
    tuple: (u32, u32, u32, u32),
    /// Health factor right after the borrow, in WAD.
    health_factor: i128,
    /// Whether the account is liquidatable 5 s later.
    liquidatable_after_5s: bool,
}

/// ALICE borrows USDC until her debt equals the limit the view reports.
fn borrow_to_the_reported_limit(t: &mut LendingTest, account: u64) -> AtTheReportedLimit {
    let limit_wad = t.ctrl_client().get_ltv_collateral_usd(&account);
    let headroom_wad = limit_wad - t.total_debt_raw(ALICE);
    assert_eq!(headroom_wad % WAD_PER_USDC_UNIT, 0, "whole USDC units");
    t.borrow_raw(ALICE, "USDC", headroom_wad / WAD_PER_USDC_UNIT);
    assert_eq!(t.total_debt_raw(ALICE), limit_wad);

    let tuple = stored_wbtc_tuple(t, account);
    let health_factor = t.health_factor_raw(ALICE);
    t.advance_time(5);
    AtTheReportedLimit {
        limit_wad,
        tuple,
        health_factor,
        liquidatable_after_5s: t.can_be_liquidated(ALICE),
    }
}

/// ALICE holds 0.5 WBTC (30,000 USD) stamped (7_500, 8_000, 500, 1_200) with
/// no debt. WBTC is relisted at `listing`, then ALICE borrows the limit.
fn borrow_the_reported_limit_after_listing(listing: (u32, u32, u32, u32)) -> AtTheReportedLimit {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
    t.supply(BOB, "USDC", 100_000.0);
    t.supply(ALICE, "WBTC", 0.5);
    let account = t.account_id(ALICE);
    assert_eq!(stored_wbtc_tuple(&t, account), (7_500, 8_000, 500, 1_200));
    list_wbtc(&t, listing);
    borrow_to_the_reported_limit(&mut t, account)
}

/// The stored threshold stays 8_000 and the listed LTV stops 500 BPS below
/// it: limit 22,500 USD, health factor 24,000 / 22,500.
const HELD_AT_THE_STORED_TUPLE: AtTheReportedLimit = AtTheReportedLimit {
    limit_wad: 22_500 * WAD,
    tuple: (7_500, 8_000, 500, 1_200),
    health_factor: 1_066_666_666_666_666_666,
    liquidatable_after_5s: false,
};

// A listing that raises LTV and threshold but also favors liquidators cannot
// move the stored threshold in a borrow. The LTV then stays the listing's
// `threshold - LTV` gap below the stored threshold.
#[test]
fn a_looser_listing_with_a_higher_bonus_keeps_the_borrow_limit_below_liquidation() {
    assert_eq!(
        borrow_the_reported_limit_after_listing((8_000, 8_500, 600, 1_200)),
        HELD_AT_THE_STORED_TUPLE
    );
}

#[test]
fn a_listing_ltv_above_the_stored_threshold_keeps_the_borrow_limit_below_liquidation() {
    assert_eq!(
        borrow_the_reported_limit_after_listing((8_200, 8_700, 600, 1_200)),
        HELD_AT_THE_STORED_TUPLE
    );
}

#[test]
fn a_looser_listing_with_a_lower_fee_keeps_the_borrow_limit_below_liquidation() {
    assert_eq!(
        borrow_the_reported_limit_after_listing((8_000, 8_500, 500, 1_000)),
        HELD_AT_THE_STORED_TUPLE
    );
}

#[test]
fn a_narrower_listing_gap_moves_the_ltv_up_to_that_gap_below_the_stored_threshold() {
    // Listed gap 300 BPS: LTV 8_000 - 300 = 7_700, limit 23,100 USD,
    // health factor 24,000 / 23,100.
    assert_eq!(
        borrow_the_reported_limit_after_listing((8_200, 8_500, 600, 1_200)),
        AtTheReportedLimit {
            limit_wad: 23_100 * WAD,
            tuple: (7_700, 8_000, 500, 1_200),
            health_factor: 1_038_961_038_961_038_961,
            liquidatable_after_5s: false,
        }
    );
}

#[test]
fn a_supply_that_cannot_take_the_listed_tuple_keeps_ltv_below_the_stored_threshold() {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
    t.supply(BOB, "USDC", 100_000.0);
    list_wbtc(&t, (7_800, 8_000, 500, 1_200));
    t.supply(ALICE, "WBTC", 0.5);
    let account = t.account_id(ALICE);
    t.borrow(ALICE, "USDC", 23_400.0);
    assert_eq!(stored_wbtc_tuple(&t, account), (7_800, 8_000, 500, 1_200));

    // The higher bonus favors liquidators, and the health factor with the
    // listed threshold is 0.81 * 30,060 / 23,400 = 1.0405, below the 1.05
    // gate. The supply keeps the stored threshold, bonus and fees.
    list_wbtc(&t, (8_000, 8_100, 600, 1_200));
    t.supply(ALICE, "WBTC", 0.001);
    assert_eq!(stored_wbtc_tuple(&t, account), (7_900, 8_000, 500, 1_200));

    // Limit 0.79 * 30,060 = 23,747.4 USD against a liquidation line of
    // 0.80 * 30,060 = 24,048 USD.
    assert_eq!(
        borrow_to_the_reported_limit(&mut t, account),
        AtTheReportedLimit {
            limit_wad: 237_474 * WAD / 10,
            tuple: (7_900, 8_000, 500, 1_200),
            health_factor: 1_012_658_227_848_101_265,
            liquidatable_after_5s: false,
        }
    );
}

#[test]
fn a_listing_edit_that_favors_liquidators_restamps_only_ltv_on_borrow() {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
    t.supply(BOB, "USDC", 100_000.0);
    t.supply(ALICE, "WBTC", 0.5);
    let account = t.account_id(ALICE);
    t.edit_asset_config("WBTC", |c| {
        c.loan_to_value = 7_000;
        c.liquidation_threshold = 7_600;
    });

    t.borrow(ALICE, "USDC", 1_000.0);

    assert_eq!(stored_wbtc_tuple(&t, account), (7_000, 8_000, 500, 1_200));
}
