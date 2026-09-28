use common::types::SeizeMode;
use controller::constants::WAD;
use soroban_sdk::testutils::Events;
use soroban_sdk::{token, vec, Address, Error};
use test_harness::errors;

use crate::liqvid_listing_params::{setup_with_threshold, Params};
use crate::redteam_liqvid_calibration::event_fields;
use crate::shared::data_for_topic;

const LT: u32 = 5_300;
const USDC_UNIT: i128 = 10_000_000;
const NAV_STEP: i128 = 10_000_000_000;
type Outcome = (i128, i128, i128, i128, i128);
const LADDER: [(i128, Option<Outcome>); 6] = [
    (943_396_230_000_000_000, None),
    (
        943_396_220_000_000_000,
        Some((688, 688, 6_072_760_099, 43_927_239_901, 0)),
    ),
    (
        525_000_000_000_000_000,
        Some((500, 10_000, 50_000_000_000, 0, 0)),
    ),
    (
        524_999_990_000_000_000,
        Some((499, 10_000, 50_000_000_000, 0, 0)),
    ),
    (
        500_000_000_000_000_000,
        Some((0, 10_000, 50_000_000_000, 0, 0)),
    ),
    (
        499_999_990_000_000_000,
        Some((500, 10_000, 47_619_046_666, 0, 238_095_333_400_000_000_000)),
    ),
];
const LADDER_HF_AFTER: i128 = 1_059_934_559_702_032_757;
const SALES: [(i128, i128, i128, i128, i128, i128); 8] = [
    (
        2,
        943,
        689,
        8_822_153_616,
        1_177_846_384,
        4_243_252_828_120_920_732,
    ),
    (
        3,
        943,
        689,
        8_822_153_616,
        6_177_846_384,
        1_618_007_211_362_217_646,
    ),
    (
        4,
        943,
        689,
        8_822_153_616,
        11_177_846_384,
        1_341_376_458_837_547_037,
    ),
    (
        5,
        943,
        689,
        8_822_153_616,
        16_177_846_384,
        1_235_739_265_009_453_189,
    ),
    (
        2,
        849,
        1_000,
        7_718_181_819,
        2_281_818_181,
        1_971_980_080_388_359_391,
    ),
    (
        3,
        849,
        1_000,
        7_718_181_819,
        7_281_818_181,
        1_235_872_659_314_892_059,
    ),
    (
        4,
        849,
        1_000,
        7_718_181_819,
        12_281_818_181,
        1_099_112_509_325_625_555,
    ),
    (
        5,
        849,
        1_000,
        7_718_181_819,
        17_281_818_181,
        1_041_487_638_134_525_979,
    ),
];
const RESIDUE_DEBT: i128 = 1_177_846_384;
const RESIDUE_STEPS: [(i128, i128, i128, i128); 7] = [
    (222, 691, 1_177_846_384, 0),
    (200, 1_000, 1_177_846_384, 0),
    (130, 1_000, 1_177_846_384, 0),
    (125, 612, 1_177_846_384, 0),
    (120, 188, 1_177_846_384, 0),
    (110, 500, 1_047_619_047, 13_022_733_700_000_000_000),
    (5, 500, 47_619_047, 113_022_733_700_000_000_000),
];
const RESIDUE_EDGE_CENTRES: [i128; 2] = [129_563_102_240_000_000_000, 123_673_870_320_000_000_000];
const BAD_DEBT: [(i128, i128, i128, i128, i128); 3] = [
    (
        500,
        0,
        50_000_000_000,
        0,
        1_000_000_000_000_000_000_000_000_000,
    ),
    (
        495,
        500,
        47_142_857_142,
        285_714_285_800_000_000_000,
        999_714_285_714_200_000_000_000_000,
    ),
    (
        400,
        500,
        38_095_238_095,
        1_190_476_190_500_000_000_000,
        998_523_809_523_699_999_999_999_999,
    ),
];
const MIN_BORROWERS: [(i128, i128, i128, i128, i128); 6] = [
    (100, 10, 689, 6, 50_000_000),
    (700, 2, 689, 1, 61_755_076),
    (2_500, 2, 689, 1, 220_553_841),
    (10_000, 2, 689, 1, 882_215_362),
    (50_000, 2, 689, 1, 4_411_076_808),
    (100_000, 2, 689, 1, 8_822_153_616),
];

struct Settled {
    bonus: i128,
    got: i128,
    paid: i128,
    bad_debt_usd: i128,
}

fn liquidator(p: &mut Params) -> Address {
    let who = p.user("rt-liquidator");
    p.allow(&who);
    who
}

fn set_nav(p: &mut Params, nav: i128) {
    p.post_nav(nav);
    if nav < p.nav_ref * 849 / 1_000 || nav > p.nav_ref * 1_030 / 1_000 {
        p.nav_ref = nav;
        p.configure_oracle(p.nav_oracle())
            .expect("band around the new NAV");
    }
}

fn open(p: &mut Params, name: &str, units: i128, nav: i128) -> u64 {
    let id = p.try_supply(name, units).expect("supply");
    let max_raw = units * nav * 5_000 / 10_000 / (WAD / USDC_UNIT);
    p.try_borrow(name, id, max_raw)
        .expect("borrow at the LTV limit");
    id
}

fn units(p: &Params, id: u64) -> i128 {
    if !p.t.ctrl_client().account_exists(&id) {
        return 0;
    }
    p.t.ctrl_client().get_collateral_amount(&id, &p.liq_key())
}

fn liquidate(p: &mut Params, id: u64, offer: i128, mode: SeizeMode) -> Result<Settled, Error> {
    let who = liquidator(p);
    let payments = vec![&p.t.env, (p.usdc_key(), offer)];
    let bonus =
        p.t.ctrl_client()
            .try_get_liquidation_estimate(&id, &payments, &mode)
            .map_or(-1, |e| e.map_or(-1, |e| e.bonus_rate_bps));
    p.t.resolve_market("USDC").token_admin.mint(&who, &offer);
    let usdc = token::Client::new(&p.t.env, &p.usdc);
    let liq = token::Client::new(&p.t.env, &p.liq);
    let (usdc_before, liq_before) = (usdc.balance(&who), liq.balance(&who));
    let receiver = match p.t.ctrl_client().try_liquidate(&who, &id, &payments, &mode) {
        Ok(Ok(receiver)) => receiver,
        Ok(Err(e)) => panic!("conversion error {e:?}"),
        Err(e) => return Err(e.expect("contract error")),
    };
    let events = p.t.env.events().all();
    let bad_debt_usd = data_for_topic(&events, "debt", "bad_debt")
        .first()
        .map_or(0, |e| event_fields(e)["total_borrow_usd_wad"]);
    let got = if receiver == 0 {
        liq.balance(&who) - liq_before
    } else {
        units(p, receiver)
    };
    Ok(Settled {
        bonus,
        got,
        paid: usdc_before - usdc.balance(&who),
        bad_debt_usd,
    })
}

#[test]
fn redteam_liqvid_ladder_boundaries_at_eight_decimal_navs() {
    let mut p = setup_with_threshold(100, LT);
    let ids: Vec<u64> = (0..LADDER.len())
        .map(|i| open(&mut p, &format!("ladder-{i}"), 10_000, WAD))
        .collect();
    for (id, (nav, want)) in ids.into_iter().zip(LADDER) {
        set_nav(&mut p, nav);
        let Some((bonus, got, paid, debt_after, bad_debt_usd)) = want else {
            assert!(!p.t.ctrl_client().is_liquidatable(&id), "NAV {nav}");
            continue;
        };
        let offer = p.debt_raw(id);
        let s = liquidate(&mut p, id, offer, SeizeMode::Transfer).unwrap();
        assert_eq!(
            (s.bonus, s.got, s.paid, s.bad_debt_usd),
            (bonus, got, paid, bad_debt_usd),
            "NAV {nav}"
        );
        if debt_after > 0 {
            assert_eq!(p.debt_raw(id), debt_after, "NAV {nav}");
            assert_eq!(p.t.ctrl_client().get_health_factor(&id), LADDER_HF_AFTER);
        } else {
            assert!(!p.t.ctrl_client().account_exists(&id), "NAV {nav}");
        }
    }
}

#[test]
fn redteam_liqvid_one_share_sales_pin_exact_payments() {
    let mut p = setup_with_threshold(100_000, LT);
    let r = p.nav_ref;
    let ids: Vec<u64> = SALES
        .iter()
        .map(|(k, pm, ..)| open(&mut p, &format!("sale-{k}-{pm}"), *k, r))
        .collect();
    for (id, (k, per_mille, bonus, paid, debt_after, hf_after)) in ids.into_iter().zip(SALES) {
        set_nav(&mut p, r * per_mille / 1_000);
        let offer = p.debt_raw(id);
        let s = liquidate(&mut p, id, offer, SeizeMode::Transfer).unwrap();
        assert_eq!(
            (s.bonus, s.got, s.paid),
            (bonus, 1, paid),
            "{k} shares at {per_mille}"
        );
        assert_eq!(
            (
                units(&p, id),
                p.debt_raw(id),
                p.t.ctrl_client().get_health_factor(&id)
            ),
            (k - 1, debt_after, hf_after),
            "{k} shares at {per_mille}"
        );
    }
}

fn residues(p: &mut Params, n: usize, tag: &str) -> Vec<u64> {
    let r = p.nav_ref;
    let ids: Vec<u64> = (0..n)
        .map(|i| open(p, &format!("{tag}-{i}"), 2, r))
        .collect();
    set_nav(p, r * 943 / 1_000);
    for id in &ids {
        let offer = p.debt_raw(*id);
        liquidate(p, *id, offer, SeizeMode::Transfer).unwrap();
        assert_eq!((units(p, *id), p.debt_raw(*id)), (1, RESIDUE_DEBT));
    }
    ids
}

#[test]
fn redteam_liqvid_residue_one_share_pays_for_all_debt_up_to_88_percent() {
    let mut p = setup_with_threshold(100_000, LT);
    let ids = residues(&mut p, RESIDUE_STEPS.len(), "residue");
    set_nav(&mut p, 223 * WAD);
    assert!(ids.iter().all(|id| !p.t.ctrl_client().is_liquidatable(id)));
    for (i, (id, (nav_usd, bonus, paid, bad_debt_usd))) in
        ids.into_iter().zip(RESIDUE_STEPS).enumerate()
    {
        set_nav(&mut p, nav_usd * WAD);
        let mode = if i % 2 == 1 {
            SeizeMode::Credit(0)
        } else {
            SeizeMode::Transfer
        };
        let s = liquidate(&mut p, id, RESIDUE_DEBT, mode).unwrap();
        assert_eq!(
            (s.bonus, s.got, s.paid, s.bad_debt_usd),
            (bonus, 1, paid, bad_debt_usd),
            "NAV ${nav_usd}"
        );
        assert!(!p.t.ctrl_client().account_exists(&id), "NAV ${nav_usd}");
        if nav_usd == 222 {
            let effective_bps = nav_usd * WAD * 10_000 / (s.paid * (WAD / USDC_UNIT)) - 10_000;
            assert_eq!(effective_bps, 8_847);
        }
    }
}

#[test]
fn redteam_liqvid_residue_edge_has_no_margin_band() {
    let mut p = setup_with_threshold(100_000, LT);
    let ids = residues(&mut p, 42, "edge");
    let navs = RESIDUE_EDGE_CENTRES
        .iter()
        .flat_map(|c| (-10..=10).map(move |step| c + step * NAV_STEP));
    for (id, nav) in ids.into_iter().zip(navs) {
        set_nav(&mut p, nav);
        assert!(p.t.ctrl_client().is_liquidatable(&id), "NAV {nav}");
        let s = liquidate(&mut p, id, RESIDUE_DEBT, SeizeMode::Transfer)
            .unwrap_or_else(|e| panic!("NAV {nav}: {e:?}"));
        assert_eq!((s.got, s.paid), (1, RESIDUE_DEBT), "NAV {nav}");
        assert!(!p.t.ctrl_client().account_exists(&id), "NAV {nav}");
    }
}

#[test]
fn redteam_liqvid_gap_below_half_nav_is_dark_then_socializes_bad_debt() {
    let mut p = setup_with_threshold(100, LT);
    p.t.supply("lender", "USDC", 1_000_000.0);
    let ids: Vec<u64> = BAD_DEBT
        .iter()
        .map(|(pm, ..)| open(&mut p, &format!("gap-{pm}"), 10_000, WAD))
        .collect();
    p.post_nav(WAD * 495 / 1_000);
    let offer = p.debt_raw(ids[1]);
    assert_eq!(
        liquidate(&mut p, ids[1], offer, SeizeMode::Transfer).err(),
        Some(Error::from_contract_error(errors::SANITY_BOUND_VIOLATED))
    );
    let usdc_key = p.usdc_key();
    for (id, (per_mille, bonus, paid, bad_debt_usd, supply_index)) in ids.into_iter().zip(BAD_DEBT)
    {
        set_nav(&mut p, WAD * per_mille / 1_000);
        let offer = p.debt_raw(id) * 3 / 2;
        let s = liquidate(&mut p, id, offer, SeizeMode::Transfer).unwrap();
        assert_eq!(
            (s.bonus, s.got, s.paid, s.bad_debt_usd),
            (bonus, 10_000, paid, bad_debt_usd),
            "NAV {per_mille}"
        );
        assert!(!p.t.ctrl_client().account_exists(&id));
        assert_eq!(
            p.t.ctrl_client().get_market_index(&usdc_key).supply_index,
            supply_index,
            "NAV {per_mille}"
        );
    }
}

#[test]
fn redteam_liqvid_minimum_borrower_per_nav_liquidates_below_hf_one() {
    for (cents, k_min, bonus, got, paid) in MIN_BORROWERS {
        let mut p = setup_with_threshold(cents, LT);
        let r = p.nav_ref;
        let below = p.try_supply("below", k_min - 1).unwrap();
        assert_eq!(
            p.try_borrow("below", below, 1),
            Err(Error::from_contract_error(
                errors::MIN_BORROW_COLLATERAL_NOT_MET
            )),
            "{k_min} - 1 shares at {cents} cents"
        );
        let id = open(&mut p, "min", k_min, r);
        set_nav(&mut p, r * 943 / 1_000);
        let offer = p.debt_raw(id);
        let s = liquidate(&mut p, id, offer, SeizeMode::Transfer).unwrap();
        assert_eq!(
            (s.bonus, s.got, s.paid),
            (bonus, got, paid),
            "{cents} cents"
        );
    }
}
