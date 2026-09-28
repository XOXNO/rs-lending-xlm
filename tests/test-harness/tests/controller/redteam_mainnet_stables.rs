use common::errors::OracleError;
use common::types::SeizeMode;
use controller::constants::WAD;
use test_harness::errors::codes::HEALTH_FACTOR_TOO_HIGH;
use test_harness::mainnet::{
    mainnet_market, LiquidationVector, MainnetSpoke, SeizedLeg, VectorOutcome,
};
use test_harness::{assert_contract_error, ALICE, LIQUIDATOR};

const SANITY_BOUND_VIOLATED: u32 = OracleError::SanityBoundViolated as u32;
const EURC_WIDENED_MAX_SANITY_WAD: i128 = 1_400_000_000_000_000_000;

const V1: LiquidationVector = LiquidationVector {
    name: "V1",
    spoke: 4,
    open_prices: &[("USDC", 1000000000000000000), ("EURC", 1137200000000000000)],
    collateral: &[("USDC", 1000000000000)],
    debt: &[("EURC", 770000000000)],
    prices: &[("USDC", 1000000000000000000), ("EURC", 1200000000000000000)],
    hf_wad: 995670995670995670,
    bonus_bps: 452,
    repay_usd_wad: 38482607504880000000000,
    repaid: &[("EURC", 320688395874)],
    seized: &[SeizedLeg {
        asset: "USDC",
        gross: 402220213641,
        to_liquidator: 400480799782,
        fee: 1739413859,
    }],
    post_collateral_usd_wad: 59777978635900000000000,
    post_weighted_usd_wad: 54995740345028000000000,
    post_debt_usd_wad: 53917392495120000000000,
    outcome: VectorOutcome::Open {
        hf_wad: 1020000000000103862,
    },
};

const V2: LiquidationVector = LiquidationVector {
    name: "V2",
    spoke: 4,
    open_prices: &[
        ("USDC", 1000000000000000000),
        ("PYUSD", 1000000000000000000),
    ],
    collateral: &[("USDC", 1000000000000)],
    debt: &[("PYUSD", 875000000000)],
    prices: &[("USDC", 951000000000000000), ("PYUSD", 1000000000000000000)],
    hf_wad: 999908571428571428,
    bonus_bps: 443,
    repay_usd_wad: 29673891027000000000000,
    repaid: &[("PYUSD", 296738910270)],
    seized: &[SeizedLeg {
        asset: "USDC",
        gross: 325851150362,
        to_liquidator: 324468865008,
        fee: 1382285354,
    }],
    post_collateral_usd_wad: 64111555600573800000000,
    post_weighted_usd_wad: 58982631152527896000000,
    post_debt_usd_wad: 57826108973000000000000,
    outcome: VectorOutcome::Open {
        hf_wad: 1020000000001174140,
    },
};

const V3: LiquidationVector = LiquidationVector {
    name: "V3",
    spoke: 4,
    open_prices: &[("EURC", 1137200000000000000), ("USDC", 1000000000000000000)],
    collateral: &[("EURC", 1000000000000)],
    debt: &[("USDC", 950000000000)],
    prices: &[("EURC", 1061000000000000000), ("USDC", 1000000000000000000)],
    hf_wad: 982821052631578947,
    bonus_bps: 563,
    repay_usd_wad: 39046608295800000000000,
    repaid: &[("USDC", 390466082958)],
    seized: &[SeizedLeg {
        asset: "EURC",
        gross: 388736402854,
        to_liquidator: 386664466901,
        fee: 2071935953,
    }],
    post_collateral_usd_wad: 64855067657190600000000,
    post_weighted_usd_wad: 57072459538327728000000,
    post_debt_usd_wad: 55953391704200000000000,
    outcome: VectorOutcome::Open {
        hf_wad: 1020000000000781507,
    },
};

const V4: LiquidationVector = LiquidationVector {
    name: "V4",
    spoke: 4,
    open_prices: &[("USST", 1009300000000000000), ("USDC", 1000000000000000000)],
    collateral: &[("USST", 100000000000000000000000)],
    debt: &[("USDC", 800000000000)],
    prices: &[("USST", 921000000000000000), ("USDC", 1000000000000000000)],
    hf_wad: 978562500000000000,
    bonus_bps: 738,
    repay_usd_wad: 30903328050800000000000,
    repaid: &[("USDC", 309033280508)],
    seized: &[SeizedLeg {
        asset: "USST",
        gross: 36030394854450640608035,
        to_liquidator: 35782765580818823018459,
        fee: 247629273631817589576,
    }],
    post_collateral_usd_wad: 58916006339050960000000,
    post_weighted_usd_wad: 50078605388193315999999,
    post_debt_usd_wad: 49096671949200000000000,
    outcome: VectorOutcome::Open {
        hf_wad: 1020000000000189748,
    },
};

const V5: LiquidationVector = LiquidationVector {
    name: "V5",
    spoke: 4,
    open_prices: &[("USDC", 1000000000000000000), ("EURC", 1137200000000000000)],
    collateral: &[("USDC", 1000000000000)],
    debt: &[("EURC", 770000000000)],
    prices: &[("USDC", 1000000000000000000), ("EURC", 1320000000000000000)],
    hf_wad: 905155450609996064,
    bonus_bps: 400,
    repay_usd_wad: 96153846153816000000000,
    repaid: &[("EURC", 728438228438)],
    seized: &[SeizedLeg {
        asset: "USDC",
        gross: 1000000000000,
        to_liquidator: 996153846154,
        fee: 3846153846,
    }],
    post_collateral_usd_wad: 0,
    post_weighted_usd_wad: 0,
    post_debt_usd_wad: 5486153846184000000000,
    outcome: VectorOutcome::BadDebt {
        socialized: &[("EURC", 41561771562)],
    },
};

const V6B: LiquidationVector = LiquidationVector {
    name: "V6b",
    spoke: 4,
    open_prices: &[("USDY", 1147700000000000000), ("USDC", 1000000000000000000)],
    collateral: &[("USDY", 1000000000000)],
    debt: &[("USDC", 918160000000)],
    prices: &[("USDY", 1080000000000000000), ("USDC", 1000000000000000000)],
    hf_wad: 999825738433388516,
    bonus_bps: 616,
    repay_usd_wad: 15745664739900000000000,
    repaid: &[("USDC", 157456647399)],
    seized: &[SeizedLeg {
        asset: "USDY",
        gross: 154774052665,
        to_liquidator: 153875966603,
        fee: 898086062,
    }],
    post_collateral_usd_wad: 91284402312180000000000,
    post_weighted_usd_wad: 77591741965353000000000,
    post_debt_usd_wad: 76070335260100000000000,
    outcome: VectorOutcome::Open {
        hf_wad: 1020000000000670432,
    },
};

fn check(v: &LiquidationVector) {
    let mut m = MainnetSpoke::open_vector(v, ALICE);
    m.assert_transfer_vector(LIQUIDATOR, ALICE, v);
}

#[test]
fn redteam_stables_v1_eur_rally_target() {
    check(&V1);
}

#[test]
fn redteam_stables_v2_usdc_inside_band_target() {
    check(&V2);
}

#[test]
fn redteam_stables_v3_eurc_last_price_before_dark() {
    check(&V3);
}

#[test]
fn redteam_stables_v4_usst_18_decimals_before_dark() {
    check(&V4);
}

#[test]
fn redteam_stables_v5_eurc_rally_insolvent_after_band_widened() {
    let mut m = MainnetSpoke::open_vector(&V5, ALICE);
    let res = m.try_liquidate(LIQUIDATOR, ALICE, V5.debt, SeizeMode::Transfer);
    assert_contract_error(res, SANITY_BOUND_VIOLATED);
    let min = mainnet_market("EURC").min_sanity_price_wad;
    m.t.seed_sanity_band("EURC", min, EURC_WIDENED_MAX_SANITY_WAD);
    m.assert_transfer_vector(LIQUIDATOR, ALICE, &V5);
}

#[test]
fn redteam_stables_v6a_usdy_at_band_floor_is_exactly_healthy() {
    let v = LiquidationVector {
        debt: &[("USDC", 918_000_000_000)],
        ..V6B
    };
    let mut m = MainnetSpoke::open_vector(&v, ALICE);
    assert_eq!(m.health_factor_raw(ALICE), WAD);
    let res = m.try_liquidate(LIQUIDATOR, ALICE, v.debt, SeizeMode::Transfer);
    assert_contract_error(res, HEALTH_FACTOR_TOO_HIGH);
}

#[test]
fn redteam_stables_v6b_usdy_at_band_floor_max_ltv_is_liquidatable() {
    check(&V6B);
}
