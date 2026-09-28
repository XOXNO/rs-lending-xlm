use common::constants::DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD;
use governance::op::AdminOperation;
use test_harness::errors::codes::MIN_BORROW_COLLATERAL_NOT_MET;
use test_harness::mainnet::{LiquidationVector, MainnetSpoke, SeizedLeg, VectorOutcome};
use test_harness::{assert_contract_error, ALICE, LIQUIDATOR};

const V1: LiquidationVector = LiquidationVector {
    name: "V1 USTRY/XLM, XLM +20%",
    spoke: 2,
    open_prices: &[("USTRY", 1076058546409975739), ("XLM", 212414871690620000)],
    collateral: &[("USTRY", 92931746449)],
    debt: &[("XLM", 282466098168)],
    prices: &[("USTRY", 1076058546409975739), ("XLM", 254897846028744000)],
    hf_wad: 972222222226202617,
    bonus_bps: 993,
    repay_usd_wad: 3364083155833535596978,
    repaid: &[("XLM", 131977700410)],
    seized: &[SeizedLeg {
        asset: "USTRY",
        gross: 34367429407,
        to_liquidator: 33994899353,
        fee: 372530054,
    }],
    post_collateral_usd_wad: 6301863386770715398971,
    post_weighted_usd_wad: 4411304370739500779279,
    post_debt_usd_wad: 3835916844083046581957,
    outcome: VectorOutcome::Open {
        hf_wad: 1150000000011469802,
    },
};

const V2: LiquidationVector = LiquidationVector {
    name: "V2 DEJTRSY(18dec)/XLM, XLM +45%",
    spoke: 3,
    open_prices: &[
        ("DEJTRSY", 1037577800000000000),
        ("XLM", 212414871690620000),
    ],
    collateral: &[("DEJTRSY", 9637831495623749852782)],
    debt: &[("XLM", 282466098170)],
    prices: &[
        ("DEJTRSY", 1037577800000000000),
        ("XLM", 308001563951399000),
    ],
    hf_wad: 804597701153061136,
    bonus_bps: 1153,
    repay_usd_wad: 8137236318229890017155,
    repaid: &[("XLM", 264194642840)],
    seized: &[SeizedLeg {
        asset: "DEJTRSY",
        gross: 8746775100355651726678,
        to_liquidator: 8638265838015007238836,
        fee: 108509262340644487842,
    }],
    post_collateral_usd_wad: 924540334278203663867,
    post_weighted_usd_wad: 647178233994742564706,
    post_debt_usd_wad: 562763681730796067641,
    outcome: VectorOutcome::Open {
        hf_wad: 1150000000007688994,
    },
};

const V3: LiquidationVector = LiquidationVector {
    name: "V3 USTRY+CETES/USDC+XLM, XLM +80%",
    spoke: 2,
    open_prices: &[
        ("USTRY", 1076058546409975739),
        ("CETES", 66519200916065103),
        ("USDC", 999990697386260000),
        ("XLM", 212414871690620000),
    ],
    collateral: &[("USTRY", 46465873224), ("CETES", 751662667491)],
    debt: &[("XLM", 141233049083), ("USDC", 30000279079)],
    prices: &[
        ("USTRY", 1076058546409975739),
        ("CETES", 66519200916065103),
        ("USDC", 999990697386260000),
        ("XLM", 382346769043116000),
    ],
    hf_wad: 833333333355114192,
    bonus_bps: 1153,
    repay_usd_wad: 7203011182926565797843,
    repaid: &[("XLM", 141233049083), ("USDC", 18030279559)],
    seized: &[
        SeizedLeg {
            asset: "USTRY",
            gross: 37328444623,
            to_liquidator: 36865361722,
            fee: 463082901,
        },
        SeizedLeg {
            asset: "CETES",
            gross: 603849584913,
            to_liquidator: 596358449921,
            fee: 7491134992,
        },
    ],
    post_collateral_usd_wad: 1966481627634509577595,
    post_weighted_usd_wad: 1376537139344156704316,
    post_debt_usd_wad: 1196988816771799745460,
    outcome: VectorOutcome::Open {
        hf_wad: 1150000000047274457,
    },
};

const V4: LiquidationVector = LiquidationVector {
    name: "V4 CETES/XLM, XLM +60%",
    spoke: 2,
    open_prices: &[("CETES", 66519200916065103), ("XLM", 212414871690620000)],
    collateral: &[("CETES", 1503325334983)],
    debt: &[("XLM", 282466098170)],
    prices: &[("CETES", 66519200916065103), ("XLM", 339863794704992000)],
    hf_wad: 729166666669488034,
    bonus_bps: 416,
    repay_usd_wad: 9599999999956843152887,
    repaid: &[("XLM", 282466098170)],
    seized: &[SeizedLeg {
        asset: "CETES",
        gross: 1503229122155,
        to_liquidator: 1496024706086,
        fee: 7204416069,
    }],
    post_collateral_usd_wad: 640000043643432322,
    post_weighted_usd_wad: 448000030550402624,
    post_debt_usd_wad: 0,
    outcome: VectorOutcome::DebtFree,
};

const V5: LiquidationVector = LiquidationVector {
    name: "V5 DEJAAA/XLM, XLM +80%",
    spoke: 3,
    open_prices: &[("DEJAAA", 1049374500000000000), ("XLM", 212414871690620000)],
    collateral: &[("DEJAAA", 9529486374978618214946)],
    debt: &[("XLM", 282466098170)],
    prices: &[("DEJAAA", 1049374500000000000), ("XLM", 382346769043116000)],
    hf_wad: 648148148151068258,
    bonus_bps: 600,
    repay_usd_wad: 9433962264122120497791,
    repaid: &[("XLM", 246738380652)],
    seized: &[SeizedLeg {
        asset: "DEJAAA",
        gross: 9529486374978618214946,
        to_liquidator: 9464757788277354233151,
        fee: 64728586701263981795,
    }],
    post_collateral_usd_wad: 0,
    post_weighted_usd_wad: 0,
    post_debt_usd_wad: 1366037735829222124421,
    outcome: VectorOutcome::BadDebt {
        socialized: &[("XLM", 35727717518)],
    },
};

const V6: LiquidationVector = LiquidationVector {
    name: "V6 DEJAAA/XLM $6 open, XLM +80%",
    spoke: 3,
    open_prices: &[("DEJAAA", 1049374500000000000), ("XLM", 212414871690620000)],
    collateral: &[("DEJAAA", 5717691824987170928)],
    debt: &[("XLM", 169479657)],
    prices: &[("DEJAAA", 1049374500000000000), ("XLM", 382346769043116000)],
    hf_wad: 648148155424966492,
    bonus_bps: 600,
    repay_usd_wad: 5660377343515866694,
    repaid: &[("XLM", 148043028)],
    seized: &[SeizedLeg {
        asset: "DEJAAA",
        gross: 5717691824987170928,
        to_liquidator: 5678854671255975825,
        fee: 38837153731195103,
    }],
    post_collateral_usd_wad: 0,
    post_weighted_usd_wad: 0,
    post_debt_usd_wad: 819622583732583408,
    outcome: VectorOutcome::BadDebt {
        socialized: &[("XLM", 21436629)],
    },
};

fn check(v: &LiquidationVector) {
    let mut m = MainnetSpoke::open_vector(v, ALICE);
    m.assert_transfer_vector(LIQUIDATOR, ALICE, v);
}

#[test]
fn redteam_rwa_v1_ustry_xlm_rally_target() {
    check(&V1);
}

#[test]
fn redteam_rwa_v2_dejtrsy_18_decimals_xlm_rally_target() {
    check(&V2);
}

#[test]
fn redteam_rwa_v3_ustry_cetes_blend_two_debt_legs() {
    check(&V3);
}

#[test]
fn redteam_rwa_v4_cetes_xlm_rally_band_closes_debt() {
    check(&V4);
}

#[test]
fn redteam_rwa_v5_dejaaa_insolvent_cleans_bad_debt() {
    check(&V5);
}

fn set_min_borrow_collateral(m: &MainnetSpoke, floor_wad: i128) {
    m.t.gov_client().execute_immediate(
        &m.t.admin,
        &AdminOperation::SetMinBorrowCollateralUsd(floor_wad),
    );
}

#[test]
fn redteam_rwa_v6_dejaaa_six_dollar_account_cleans_bad_debt() {
    let mut m = MainnetSpoke::build(V6.spoke);
    m.set_prices(V6.open_prices);
    m.supply(ALICE, "DEJAAA", V6.collateral[0].1);
    assert_contract_error(
        m.try_borrow(ALICE, "XLM", V6.debt[0].1),
        MIN_BORROW_COLLATERAL_NOT_MET,
    );
    set_min_borrow_collateral(&m, 0);
    m.borrow(ALICE, "XLM", V6.debt[0].1);
    set_min_borrow_collateral(&m, DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD);
    m.set_prices(V6.prices);
    m.assert_transfer_vector(LIQUIDATOR, ALICE, &V6);
}
