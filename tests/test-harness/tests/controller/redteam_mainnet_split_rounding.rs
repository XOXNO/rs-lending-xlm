use common::constants::BPS;
use common::errors::CollateralError;
use common::math::fp_core::{mul_div_ceil, mul_div_floor};
use common::types::SeizeMode;
use controller::constants::{RAY, WAD};
use test_harness::mainnet::{LiquidationObservation, MainnetSpoke};
use test_harness::{assert_contract_error, ALICE, LIQUIDATOR};

const XLM_UNITS: i128 = 500_000_000_000;
const XLM_OPEN_WAD: i128 = 200_000_000_000_000_000;
const DEBT_UNITS: i128 = 75_000_000_000;

fn xlm_usdc_book(open_xlm_wad: i128, debt: i128, xlm_wad: i128) -> MainnetSpoke {
    let mut m = MainnetSpoke::build(1);
    m.set_prices(&[("XLM", open_xlm_wad), ("USDC", WAD)]);
    m.open_position(ALICE, &[("XLM", XLM_UNITS)], &[("USDC", debt)]);
    m.set_price("XLM", xlm_wad);
    m
}

fn liquidate(m: &mut MainnetSpoke, usdc: i128, mode: SeizeMode) -> LiquidationObservation {
    m.observe_liquidation(LIQUIDATOR, ALICE, &[("USDC", usdc)], mode)
        .expect("liquidation")
}

fn quote_units(m: &MainnetSpoke) -> i128 {
    let offer = m.debt_raw(ALICE, "USDC");
    let estimate = m.estimate(ALICE, &[("USDC", offer)], SeizeMode::Transfer);
    offer - estimate.refunds.iter().map(|r| r.amount).sum::<i128>()
}

fn band_witness() -> MainnetSpoke {
    xlm_usdc_book(260_000_000_000_000_000, 97_500_000_000, XLM_OPEN_WAD)
}

fn band_seizure_bound_units(m: &MainnetSpoke, repay_usd_wad: i128, slices: i128) -> i128 {
    let env = &m.t.env;
    let per_bps = mul_div_ceil(env, repay_usd_wad, 10_000_000, BPS * m.price("XLM"));
    per_bps + slices
}

fn band_split(mode_first: SeizeMode, credit: bool) {
    let unit = 10i128.pow(20);

    let mut single = band_witness();
    assert_eq!(single.health_factor_raw(ALICE), 800_000_000_000_000_000);
    let one = liquidate(&mut single, 97_500_000_000, mode_first);
    assert_eq!(one.bonus_bps, 256);
    assert_eq!(one.repaid_usd_wad, 9_750_000_000_000_000_000_000);
    assert_eq!(one.shares_burned["XLM"], 499_980_000_000 * unit);
    assert_eq!(one.revenue_delta["XLM"], 1_497_600_000 * unit);
    assert!(one.debt_free);

    let mut chain = band_witness();
    let first = liquidate(&mut chain, 97_402_500_000, mode_first);
    assert_eq!(first.bonus_bps, 256);
    assert_eq!(first.shares_burned["XLM"], 499_480_020_000 * unit);
    assert_eq!(first.revenue_delta["XLM"], 1_496_102_400 * unit);
    assert!(!first.debt_free);
    let rest = chain.debt_raw(ALICE, "USDC");
    assert_eq!(rest, 97_500_000);
    let second_mode = if credit {
        SeizeMode::Credit(first.receiver_id)
    } else {
        SeizeMode::Transfer
    };
    let second = liquidate(&mut chain, rest, second_mode);
    assert_eq!(second.bonus_bps, 666);
    assert_eq!(second.shares_burned["XLM"], 519_967_500 * unit);
    assert_eq!(second.revenue_delta["XLM"], 3_896_100 * unit);
    assert!(second.debt_free);
    assert!(second.bonus_bps > first.bonus_bps);

    let single_burned = one.shares_burned["XLM"];
    let chain_burned = first.shares_burned["XLM"] + second.shares_burned["XLM"];
    assert_eq!(chain_burned - single_burned, 19_987_500 * unit);
    let bound = band_seizure_bound_units(&chain, one.repaid_usd_wad, 2);
    assert_eq!(bound, 48_750_002);
    assert!(chain_burned - single_burned <= bound * unit);

    if credit {
        assert_eq!(
            one.receiver_shares["XLM"] + one.revenue_delta["XLM"],
            one.shares_burned["XLM"]
        );
        assert_eq!(one.receiver_shares["XLM"], 498_482_400_000 * unit);
        assert_eq!(second.receiver_id, first.receiver_id);
        assert_eq!(
            second.receiver_shares["XLM"],
            (497_983_917_600 + 516_071_400) * unit
        );
    } else {
        assert_eq!(one.liquidator_delta["XLM"], 498_482_400_000);
        assert_eq!(first.liquidator_delta["XLM"], 497_983_917_600);
        assert_eq!(second.liquidator_delta["XLM"], 516_071_400);
    }
    single.t.assert_spoke_usage_matches_positions();
    chain.t.assert_spoke_usage_matches_positions();
}

#[test]
fn redteam_split_band_extra_seizure_is_below_one_bps_transfer() {
    band_split(SeizeMode::Transfer, false);
}

#[test]
fn redteam_split_band_extra_seizure_is_below_one_bps_credit() {
    band_split(SeizeMode::Credit(0), true);
}

#[test]
fn redteam_split_target_pays_less_than_one_close() {
    let mut chain = xlm_usdc_book(XLM_OPEN_WAD, DEBT_UNITS, 182_700_000_000_000_000);
    assert_eq!(chain.health_factor_raw(ALICE), 950_040_000_000_000_000);
    let quote = quote_units(&chain);
    assert_eq!(quote, 53_207_266_019);
    let half = quote / 2;

    let first = liquidate(&mut chain, half, SeizeMode::Transfer);
    assert_eq!(first.bonus_bps, 1130);
    assert_eq!(first.liquidator_delta["USDC"], 0);
    assert_eq!(first.liquidator_delta["XLM"], 160_093_586_620);
    let second = liquidate(&mut chain, half, SeizeMode::Transfer);
    assert_eq!(second.bonus_bps, 1079);
    assert!(second.bonus_bps < first.bonus_bps);
    assert_eq!(half - second.liquidator_delta["USDC"], 26_233_390_942);
    assert_eq!(second.liquidator_delta["XLM"], 157_221_148_761);
    assert_eq!(chain.health_factor_raw(ALICE), 1_150_000_000_004_904_124);

    let total = half + half - second.liquidator_delta["USDC"];
    assert_eq!(total, 52_837_023_951);
    let mut single = xlm_usdc_book(XLM_OPEN_WAD, DEBT_UNITS, 182_700_000_000_000_000);
    let one = liquidate(&mut single, total, SeizeMode::Transfer);
    assert_eq!(one.bonus_bps, 1130);
    assert_eq!(one.liquidator_delta["XLM"], 317_959_154_969);
    assert_eq!(single.health_factor_raw(ALICE), 1_145_291_407_212_844_297);

    let chain_burned = first.shares_burned["XLM"] + second.shares_burned["XLM"];
    assert_eq!(chain_burned, 321_148_425_635 * 10i128.pow(20));
    assert_eq!(one.shares_burned["XLM"], 321_880_720_621 * 10i128.pow(20));
    assert!(chain_burned <= one.shares_burned["XLM"]);
}

#[test]
fn redteam_split_target_second_half_reverts_once_healthy() {
    let mut m = xlm_usdc_book(XLM_OPEN_WAD, DEBT_UNITS, 182_700_000_000_000_000);
    let quote = quote_units(&m);
    liquidate(&mut m, quote, SeizeMode::Transfer);
    assert!(m.health_factor_raw(ALICE) >= WAD);
    let res = m.try_liquidate(
        LIQUIDATOR,
        ALICE,
        &[("USDC", quote / 2)],
        SeizeMode::Transfer,
    );
    assert_contract_error(res, CollateralError::HealthFactorTooHigh as u32);
}

#[test]
fn redteam_split_insolvent_is_additive_at_base_bonus() {
    let mut chain = xlm_usdc_book(XLM_OPEN_WAD, DEBT_UNITS, 125_000_000_000_000_000);
    assert_eq!(chain.health_factor_raw(ALICE), 650_000_000_000_000_000);
    let quote = quote_units(&chain);
    assert_eq!(quote, 57_339_449_541);
    let slice = quote / 5;
    assert_eq!(slice, 11_467_889_908);

    let unit = 10i128.pow(20);
    let mut chain_liquidator = 0;
    let mut chain_gross = 0i128;
    for k in 0..5 {
        let obs = liquidate(&mut chain, slice, SeizeMode::Transfer);
        assert_eq!(obs.bonus_bps, 900);
        assert_eq!(obs.liquidator_delta["XLM"], 99_009_174_309);
        let residual = if k == 4 { 15 } else { 0 };
        assert_eq!(obs.revenue_delta["XLM"], (990_825_688 + residual) * unit);
        assert_eq!(obs.shares_burned["XLM"], (99_999_999_997 + residual) * unit);
        assert_eq!(obs.bad_debt_event.is_some(), k == 4);
        chain_liquidator += obs.liquidator_delta["XLM"];
        chain_gross += 99_999_999_997;
    }

    let mut single = xlm_usdc_book(XLM_OPEN_WAD, DEBT_UNITS, 125_000_000_000_000_000);
    let one = liquidate(&mut single, 5 * slice, SeizeMode::Transfer);
    assert_eq!(one.bonus_bps, 900);
    assert_eq!(one.liquidator_delta["XLM"], 495_045_871_548);
    assert_eq!(one.revenue_delta["XLM"], (4_954_128_440 + 12) * unit);
    assert_eq!(one.shares_burned["XLM"], 500_000_000_000 * unit);
    assert!(one.bad_debt_event.is_some());
    assert!(chain_liquidator <= one.liquidator_delta["XLM"]);
    assert!(chain_gross <= 499_999_999_988);
    chain.t.assert_spoke_usage_matches_positions();
    single.t.assert_spoke_usage_matches_positions();
}

const SOLVBTC_UNITS: i128 = 12_500_000;
const SOLVBTC_OPEN_WAD: i128 = 80_000 * WAD;
const SOLVBTC_STRESSED_WAD: i128 = 41_142_860_000_000_000_000_000;
const SOLVBTC_DEBT_UNITS: i128 = 60_000_000_000;
const SOLVBTC_UNIT: i128 = 10_000_000_000_000_000_000;

fn solvbtc_book() -> MainnetSpoke {
    let mut m = MainnetSpoke::build(1);
    m.set_prices(&[("SolvBTC", SOLVBTC_OPEN_WAD), ("USDC", WAD)]);
    m.open_position(
        ALICE,
        &[("SolvBTC", SOLVBTC_UNITS)],
        &[("USDC", SOLVBTC_DEBT_UNITS)],
    );
    m.set_price("SolvBTC", SOLVBTC_STRESSED_WAD);
    assert_eq!(m.health_factor_raw(ALICE), 600_000_041_666_666_666);
    assert_eq!(quote_units(&m), 47_182_178_899);
    m
}

fn slice_sizes() -> Vec<i128> {
    (0..20i128)
        .map(|i| 300_000_000 * (i % 7 + 1) + 7_919 * i * i)
        .collect()
}

fn assert_pool_solvent(m: &MainnetSpoke, asset: &str) {
    let env = &m.t.env;
    let st = m.pool_state(asset);
    let assets =
        st.cash * m.share_unit(asset) + mul_div_floor(env, st.borrowed, st.borrow_index, RAY);
    let claims = mul_div_ceil(env, st.supplied, st.supply_index, RAY);
    assert!(assets >= claims, "{asset} pool: {assets} < {claims}");
}

#[test]
fn redteam_rounding_transfer_chain_drifts_at_most_one_unit_per_call() {
    let sizes = slice_sizes();
    let total: i128 = sizes.iter().sum();
    assert_eq!(total, 23_119_559_930);

    let mut chain = solvbtc_book();
    let (mut liq, mut revenue, mut burned) = (0, 0, 0);
    for size in &sizes {
        let obs = liquidate(&mut chain, *size, SeizeMode::Transfer);
        assert_eq!(obs.bonus_bps, 900);
        assert_eq!(obs.liquidator_delta["USDC"], 0);
        liq += obs.liquidator_delta["SolvBTC"];
        revenue += obs.revenue_delta["SolvBTC"];
        burned += obs.shares_burned["SolvBTC"];
        assert_pool_solvent(&chain, "SolvBTC");
        assert_pool_solvent(&chain, "USDC");
    }
    chain.t.assert_spoke_usage_matches_positions();

    let mut single = solvbtc_book();
    let one = liquidate(&mut single, total, SeizeMode::Transfer);
    assert_eq!(one.bonus_bps, 900);

    assert_eq!(liq, 6_064_386);
    assert_eq!(revenue, 60_681 * SOLVBTC_UNIT);
    assert_eq!(burned, 6_125_067 * SOLVBTC_UNIT);
    assert_eq!(one.liquidator_delta["SolvBTC"], 6_064_389);
    assert_eq!(one.revenue_delta["SolvBTC"], 60_688 * SOLVBTC_UNIT);
    assert_eq!(one.shares_burned["SolvBTC"], 6_125_077 * SOLVBTC_UNIT);

    let calls = sizes.len() as i128;
    assert!(liq <= one.liquidator_delta["SolvBTC"] + calls);
    assert!(revenue >= one.revenue_delta["SolvBTC"] - calls * SOLVBTC_UNIT);
}

#[test]
fn redteam_rounding_credit_chain_splits_every_call_exactly() {
    let sizes = slice_sizes();
    let total: i128 = sizes.iter().sum();

    let mut chain = solvbtc_book();
    let mut receiver = 0;
    let (mut revenue, mut burned, mut credited) = (0, 0, 0);
    for size in &sizes {
        let obs = liquidate(&mut chain, *size, SeizeMode::Credit(receiver));
        receiver = obs.receiver_id;
        assert_eq!(obs.bonus_bps, 900);
        let delta = obs.receiver_shares["SolvBTC"] - credited;
        assert_eq!(
            delta + obs.revenue_delta["SolvBTC"],
            obs.shares_burned["SolvBTC"]
        );
        credited = obs.receiver_shares["SolvBTC"];
        revenue += obs.revenue_delta["SolvBTC"];
        burned += obs.shares_burned["SolvBTC"];
        assert_eq!(obs.liquidator_delta["SolvBTC"], 0);
    }
    chain.t.assert_spoke_usage_matches_positions();

    let mut single = solvbtc_book();
    let one = liquidate(&mut single, total, SeizeMode::Credit(0));

    assert_eq!(credited, 60_643_885_905_005_147_178_715_587);
    assert_eq!(revenue, 606_888_406_017_471_821_284_413);
    assert_eq!(burned, 61_250_774_311_022_619_000_000_000);
    assert_eq!(
        one.receiver_shares["SolvBTC"],
        60_643_885_905_005_145_198_532_110
    );
    assert_eq!(
        one.revenue_delta["SolvBTC"],
        606_888_406_017_471_801_467_890
    );
    assert_eq!(
        one.shares_burned["SolvBTC"],
        61_250_774_311_022_617_000_000_000
    );
    assert!(burned - one.shares_burned["SolvBTC"] < SOLVBTC_UNIT);
}
