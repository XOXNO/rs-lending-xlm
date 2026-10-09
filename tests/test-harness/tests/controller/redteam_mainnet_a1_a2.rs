use std::collections::BTreeMap;

use common::types::SeizeMode;
use controller::constants::WAD;
use soroban_sdk::vec;
use test_harness::mainnet::{LiquidationObservation, MainnetSpoke};
use test_harness::{ALICE, BOB, CAROL, LIQUIDATOR};

const XLM_20C: i128 = 200_000_000_000_000_000;
const EURC_110C: i128 = 1_100_000_000_000_000_000;
const BTC_80K: i128 = 80_000 * WAD;
const XLM_UNITS: i128 = 100_000_000_000;
const XLM_SHARE: i128 = 10i128.pow(20);
const EXTRA_LEGS: [(&str, i128); 4] = [("SolvBTC", 1), ("xSolvBTC", 1), ("EURC", 1), ("USDT0", 1)];

fn a1_book() -> MainnetSpoke {
    let mut m = MainnetSpoke::build(1);
    m.set_prices(&[
        ("XLM", 2 * XLM_20C),
        ("USDC", WAD),
        ("EURC", EURC_110C),
        ("USDT0", WAD),
        ("SolvBTC", BTC_80K),
        ("xSolvBTC", BTC_80K),
    ]);
    m.open_position(
        ALICE,
        &[("XLM", XLM_UNITS)],
        &[
            ("USDC", 25_000_000_000),
            ("SolvBTC", 10_000),
            ("xSolvBTC", 10_000),
            ("EURC", 50_000_000),
            ("USDT0", 50_000_000),
        ],
    );
    m.set_price("XLM", XLM_20C);
    assert_eq!(m.health_factor_raw(ALICE), 617_454_977_241_242_826);
    let (_, plan) = m.reference_plan(ALICE, &[("USDC", 1_000_000_000_000_000)]);
    assert_eq!(
        (
            plan.totals.total_collateral,
            plan.totals.total_debt,
            plan.quote_usd,
            plan.bonus_bps
        ),
        (
            2_000 * WAD,
            2_526_500_000_000_000_000_000,
            1_834_862_385_321_100_917_431,
            900
        )
    );
    m
}

fn with_extras(usdc: i128) -> Vec<(&'static str, i128)> {
    let mut out = Vec::from([("USDC", usdc)]);
    out.extend(EXTRA_LEGS);
    out
}

fn estimate_xlm(m: &MainnetSpoke, payments: &[(&str, i128)]) -> (i128, i128, i128) {
    let e = m.estimate(ALICE, payments, SeizeMode::Transfer);
    assert_eq!(e.bonus_rate_bps, 900);
    assert_eq!(e.seized_collaterals.len(), 1);
    (
        e.max_payment_wad,
        e.seized_collaterals.get(0).unwrap().amount,
        e.protocol_fees.get(0).unwrap().amount,
    )
}

fn assert_a1_socialized(obs: &LiquidationObservation) {
    assert!(obs.debt_free);
    let expected: BTreeMap<&str, i128> = [
        ("USDC", 25_000_000_000 * XLM_SHARE),
        ("SolvBTC", 10_000 * 10i128.pow(19)),
        ("xSolvBTC", 10_000 * 10i128.pow(19)),
        ("EURC", 50_000_000 * XLM_SHARE),
        ("USDT0", 50_000_000 * XLM_SHARE),
    ]
    .into_iter()
    .collect();
    for (asset, burned) in expected {
        assert_eq!(obs.borrowed_burned[asset], burned, "{asset} fully closed");
    }
    assert_eq!(obs.liquidator_delta["USDC"], 0);
    for (asset, _) in EXTRA_LEGS {
        assert_eq!(obs.liquidator_delta[asset], 0, "{asset} leg");
    }
}

#[test]
fn redteam_a1_pay_quote_seizes_all_and_cleans_up() {
    let mut m = a1_book();
    let obs = m
        .observe_liquidation(
            LIQUIDATOR,
            ALICE,
            &[("USDC", 18_348_623_853)],
            SeizeMode::Transfer,
        )
        .expect("pay-quote liquidation");
    assert_eq!(obs.bonus_bps, 900);
    assert_eq!(obs.repaid_usd_wad, 1_834_862_385_300_000_000_000);
    assert_eq!(obs.liquidator_delta["USDC"], 0);
    assert_eq!(obs.shares_burned["XLM"], XLM_UNITS * XLM_SHARE);
    assert_eq!(obs.liquidator_delta["XLM"], 99_009_174_312);
    assert_eq!(obs.revenue_delta["XLM"], 990_825_688 * XLM_SHARE);
    assert_eq!(obs.bad_debt_event, Some((691_637_614_700_000_000_000, 0)));
    assert!(obs.debt_free);
    m.t.assert_spoke_usage_matches_positions();
}

#[test]
fn redteam_a1_one_unit_extra_legs_keep_seize_all_below_the_quote() {
    let mut m = a1_book();
    let attack = with_extras(18_348_591_849);
    assert_eq!(
        estimate_xlm(&m, &attack),
        (1_834_860_785_110_000_000_000, XLM_UNITS, 990_835_289)
    );
    let obs = m
        .observe_liquidation(LIQUIDATOR, ALICE, &attack, SeizeMode::Transfer)
        .expect("slack liquidation");
    assert_eq!(obs.bonus_bps, 900);
    assert_eq!(obs.repaid_usd_wad, 1_834_860_785_110_000_000_000);
    assert_eq!(obs.shares_burned["XLM"], XLM_UNITS * XLM_SHARE);
    assert_eq!(obs.liquidator_delta["XLM"], 99_009_164_711);
    assert_eq!(obs.revenue_delta["XLM"], 990_835_289 * XLM_SHARE);
    assert_eq!(obs.bad_debt_event, Some((691_639_214_890_000_000_000, 0)));
    assert_a1_socialized(&obs);

    let pay_quote_socialized = 691_637_614_700_000_000_000;
    assert_eq!(
        obs.bad_debt_event.unwrap().0 - pay_quote_socialized,
        1_600_190_000_000_000
    );
    assert_eq!(
        1_834_862_385_321_100_917_431 - obs.repaid_usd_wad,
        1_600_211_100_917_431
    );
    m.t.assert_spoke_usage_matches_positions();
}

#[test]
fn redteam_a1_one_unit_below_the_boundary_is_pro_rata_and_cleans_the_remainder() {
    let mut m = a1_book();
    let boundary = with_extras(18_348_591_848);
    assert_eq!(
        estimate_xlm(&m, &boundary),
        (1_834_860_785_010_000_000_000, 99_999_912_783, 990_824_823)
    );
    let obs = m
        .observe_liquidation(LIQUIDATOR, ALICE, &boundary, SeizeMode::Transfer)
        .expect("boundary liquidation");
    assert_eq!(obs.bonus_bps, 900);
    assert_eq!(obs.repaid_usd_wad, 1_834_860_785_010_000_000_000);
    assert_eq!(obs.liquidator_delta["XLM"], 99_009_087_960);
    assert_eq!(obs.shares_burned["XLM"], XLM_UNITS * XLM_SHARE);
    assert_eq!(obs.revenue_delta["XLM"], (990_824_823 + 87_217) * XLM_SHARE);
    assert_eq!(
        obs.bad_debt_event,
        Some((691_639_214_990_000_000_000, 1_744_340_000_000_000))
    );
    assert_a1_socialized(&obs);
    m.t.assert_spoke_usage_matches_positions();
}

#[test]
fn redteam_a1_over_offer_trims_the_main_leg_to_the_normal_seize_all_path() {
    let mut m = a1_book();
    let mut over: Vec<(&str, i128)> = EXTRA_LEGS.to_vec();
    over.push(("USDC", 1_000_000_000_000_000));
    assert_eq!(
        estimate_xlm(&m, &over),
        (1_834_862_385_310_000_000_000, XLM_UNITS, 990_825_688)
    );
    let obs = m
        .observe_liquidation(LIQUIDATOR, ALICE, &over, SeizeMode::Transfer)
        .expect("over-offer liquidation");
    assert_eq!(obs.repaid_usd_wad, 1_834_862_385_310_000_000_000);
    assert_eq!(
        1_834_862_385_321_100_917_431 - obs.repaid_usd_wad,
        11_100_917_431
    );
    assert_eq!(
        obs.liquidator_delta["USDC"],
        1_000_000_000_000_000 - 18_348_607_851
    );
    assert_eq!(obs.shares_burned["XLM"], XLM_UNITS * XLM_SHARE);
    assert_eq!(obs.liquidator_delta["XLM"], 99_009_174_312);
    assert_eq!(obs.revenue_delta["XLM"], 990_825_688 * XLM_SHARE);
    assert_eq!(obs.bad_debt_event, Some((691_637_614_690_000_000_000, 0)));
    for (asset, _) in EXTRA_LEGS {
        assert_eq!(obs.liquidator_delta[asset], 0, "{asset} leg kept");
    }
    m.t.assert_spoke_usage_matches_positions();
}

const USDY_UNITS: i128 = 180_000_000_000;
const PYUSD_DEBT: i128 = 200_005_000_000;
const USDY_117: i128 = 1_170_000_000_000_000_000;

fn a2_book() -> MainnetSpoke {
    let mut m = MainnetSpoke::build(4);
    m.set_prices(&[
        ("USDY", 1_320_000_000_000_000_000),
        ("PYUSD", 950_000_000_000_000_000),
    ]);
    m.open_position(ALICE, &[("USDY", USDY_UNITS)], &[("PYUSD", PYUSD_DEBT)]);
    m.set_prices(&[("USDY", USDY_117), ("PYUSD", WAD)]);
    assert_eq!(m.health_factor_raw(ALICE), 895_027_624_309_392_265);
    m.supply(CAROL, "PYUSD", 10_000_000_000);
    assert_eq!(m.pool_state("PYUSD").supplied, 10_000_000_000 * XLM_SHARE);
    let e = m.estimate(ALICE, &[("PYUSD", PYUSD_DEBT)], SeizeMode::Transfer);
    assert_eq!(e.bonus_rate_bps, 529);
    assert_eq!(e.max_payment_wad, 19_990_482_664_900_000_000_000);
    m
}

#[test]
fn redteam_a2_no_nudge_partial_close_at_cap_529() {
    let mut m = a2_book();
    let index = m.pool_state("PYUSD").supply_index;
    let obs = m
        .observe_liquidation(
            LIQUIDATOR,
            ALICE,
            &[("PYUSD", PYUSD_DEBT)],
            SeizeMode::Transfer,
        )
        .expect("no-nudge liquidation");
    assert_eq!(obs.bonus_bps, 529);
    assert_eq!(obs.repaid_usd_wad, 19_990_482_664_900_000_000_000);
    assert_eq!(obs.liquidator_delta["PYUSD"], 100_173_351);
    assert_eq!(obs.shares_burned["USDY"], 179_897_258_101 * XLM_SHARE);
    assert_eq!(obs.liquidator_delta["USDY"], 178_993_414_911);
    assert_eq!(obs.revenue_delta["USDY"], 903_843_190 * XLM_SHARE);
    assert!(!obs.debt_free);
    assert_eq!(obs.bad_debt_event, None);
    let post = obs.post.expect("residual debt stays");
    assert_eq!(
        (
            post.total_collateral,
            post.weighted_collateral,
            post.total_debt,
            post.health_factor
        ),
        (
            12_020_802_183_000_000_000,
            10_217_681_855_550_000_000,
            10_017_335_100_000_000_000,
            1_020_000_005_345_733_118
        )
    );
    assert_eq!(m.pool_state("PYUSD").supply_index, index);
    m.t.assert_spoke_usage_matches_positions();
}

#[test]
fn redteam_a2_one_unit_liquidation_repays_without_seizure() {
    let mut m = a2_book();
    let obs = m
        .observe_liquidation(LIQUIDATOR, ALICE, &[("PYUSD", 1)], SeizeMode::Transfer)
        .expect("zero-seizure liquidation");
    assert_eq!(obs.bonus_bps, 529);
    assert_eq!(obs.repaid_usd_wad, 100_000_000_000);
    assert_eq!(obs.liquidator_delta["PYUSD"], 0);
    assert_eq!(obs.shares_burned["USDY"], 0);
    assert_eq!(obs.liquidator_delta["USDY"], 0);
    assert_eq!(obs.revenue_delta["USDY"], 0);
    assert_eq!(m.debt_raw(ALICE, "PYUSD"), PYUSD_DEBT - 1);
    assert_eq!(m.health_factor_raw(ALICE), 895_027_624_313_867_291);
    m.t.assert_spoke_usage_matches_positions();
}

#[test]
fn redteam_a2_third_party_repay_lifts_cap_to_530_and_full_close() {
    let mut m = a2_book();
    let account_id = m.account_id(ALICE);
    let index = m.pool_state("PYUSD").supply_index;
    let bob = m.t.get_or_create_user(BOB);
    m.t.resolve_market("PYUSD")
        .token_admin
        .mint(&bob, &5_000_000);
    m.t.ctrl_client().repay(
        &bob,
        &account_id,
        &vec![&m.t.env, (m.hub_asset("PYUSD"), 5_000_000i128)],
    );
    let debt = m.debt_raw(ALICE, "PYUSD");
    assert_eq!(debt, 200_000_000_000);
    assert_eq!(m.health_factor_raw(ALICE), 895_050_000_000_000_000);

    let e = m.estimate(ALICE, &[("PYUSD", debt)], SeizeMode::Transfer);
    assert_eq!(e.bonus_rate_bps, 530);
    assert_eq!(e.max_payment_wad, 20_000 * WAD);

    let obs = m
        .observe_liquidation(LIQUIDATOR, ALICE, &[("PYUSD", debt)], SeizeMode::Transfer)
        .expect("nudged liquidation");
    assert_eq!(obs.bonus_bps, 530);
    assert_eq!(obs.repaid_usd_wad, 20_000 * WAD);
    assert_eq!(obs.liquidator_delta["PYUSD"], 0);
    assert_eq!(obs.shares_burned["USDY"], USDY_UNITS * XLM_SHARE);
    assert_eq!(obs.liquidator_delta["USDY"], 179_094_017_095);
    assert_eq!(obs.revenue_delta["USDY"], 905_982_905 * XLM_SHARE);
    assert!(obs.debt_free);
    assert_eq!(obs.bad_debt_event, None);
    assert!(m.supply_shares(account_id).is_empty());
    assert_eq!(m.pool_state("PYUSD").supply_index, index);
    m.t.assert_spoke_usage_matches_positions();
}
