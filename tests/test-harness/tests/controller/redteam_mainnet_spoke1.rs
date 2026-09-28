use common::types::SeizeMode;
use test_harness::errors::codes::INSUFFICIENT_LIQUIDITY;
use test_harness::mainnet::{LiquidationVector, MainnetSpoke, SeizedLeg, VectorOutcome};
use test_harness::{assert_contract_error, ALICE, BOB, LIQUIDATOR};

const V1: LiquidationVector = LiquidationVector {
    name: "V1 XLM->USDC HF<1",
    spoke: 1,
    open_prices: &[("XLM", 200000000000000000), ("USDC", 1000000000000000000)],
    collateral: &[("XLM", 500000000000)],
    debt: &[("USDC", 75000000000)],
    prices: &[("XLM", 190000000000000000), ("USDC", 1000000000000000000)],
    hf_wad: 988000000000000000,
    bonus_bps: 1087,
    repay_usd_wad: 4259959188600000000000,
    repaid: &[("USDC", 42599591886)],
    seized: &[SeizedLeg {
        asset: "XLM",
        gross: 248579829073,
        to_liquidator: 245655254986,
        fee: 2924574087,
    }],
    post_collateral_usd_wad: 4776983247613000000000,
    post_weighted_usd_wad: 3726046933138140000000,
    post_debt_usd_wad: 3240040811400000000000,
    outcome: VectorOutcome::Open {
        hf_wad: 1150000000008685075,
    },
};

const V2A: LiquidationVector = LiquidationVector {
    name: "V2a XLM->USDC capped+dust",
    spoke: 1,
    open_prices: &[("XLM", 200000000000000000), ("USDC", 1000000000000000000)],
    collateral: &[("XLM", 500000000000)],
    debt: &[("USDC", 75000000000)],
    prices: &[("XLM", 164000000000000000), ("USDC", 1000000000000000000)],
    hf_wad: 852800000000000000,
    bonus_bps: 933,
    repay_usd_wad: 7500000000000000000000,
    repaid: &[("USDC", 75000000000)],
    seized: &[SeizedLeg {
        asset: "XLM",
        gross: 499984756097,
        to_liquidator: 494864634146,
        fee: 5120121951,
    }],
    post_collateral_usd_wad: 250000009200000000,
    post_weighted_usd_wad: 195000007176000000,
    post_debt_usd_wad: 0,
    outcome: VectorOutcome::DebtFree,
};

const V2B: LiquidationVector = LiquidationVector {
    name: "V2b XLM->USDC band",
    spoke: 1,
    open_prices: &[("XLM", 200000000000000000), ("USDC", 1000000000000000000)],
    collateral: &[("XLM", 500000000000)],
    debt: &[("USDC", 75000000000)],
    prices: &[("XLM", 160000000000000000), ("USDC", 1000000000000000000)],
    hf_wad: 832000000000000000,
    bonus_bps: 666,
    repay_usd_wad: 7500000000000000000000,
    repaid: &[("USDC", 75000000000)],
    seized: &[SeizedLeg {
        asset: "XLM",
        gross: 499968750000,
        to_liquidator: 496222500000,
        fee: 3746250000,
    }],
    post_collateral_usd_wad: 500000000000000000,
    post_weighted_usd_wad: 390000000000000000,
    post_debt_usd_wad: 0,
    outcome: VectorOutcome::DebtFree,
};

const V3: LiquidationVector = LiquidationVector {
    name: "V3 XLM->USDC insolvent",
    spoke: 1,
    open_prices: &[("XLM", 200000000000000000), ("USDC", 1000000000000000000)],
    collateral: &[("XLM", 500000000000)],
    debt: &[("USDC", 75000000000)],
    prices: &[("XLM", 145000000000000000), ("USDC", 1000000000000000000)],
    hf_wad: 754000000000000000,
    bonus_bps: 900,
    repay_usd_wad: 6651376146700000000000,
    repaid: &[("USDC", 66513761467)],
    seized: &[SeizedLeg {
        asset: "XLM",
        gross: 500000000000,
        to_liquidator: 495045871559,
        fee: 4954128441,
    }],
    post_collateral_usd_wad: 0,
    post_weighted_usd_wad: 0,
    post_debt_usd_wad: 848623853300000000000,
    outcome: VectorOutcome::BadDebt {
        socialized: &[("USDC", 8486238533)],
    },
};

const V4: LiquidationVector = LiquidationVector {
    name: "V4 SolvBTC->XLM (debt up)",
    spoke: 1,
    open_prices: &[
        ("SolvBTC", 80000000000000000000000),
        ("XLM", 200000000000000000),
    ],
    collateral: &[("SolvBTC", 12500000)],
    debt: &[("XLM", 300000000000)],
    prices: &[
        ("SolvBTC", 80000000000000000000000),
        ("XLM", 234500000000000000),
    ],
    hf_wad: 995024875621890547,
    bonus_bps: 1215,
    repay_usd_wad: 2987395533643150000000,
    repaid: &[("XLM", 127394265827)],
    seized: &[SeizedLeg {
        asset: "SolvBTC",
        gross: 4187955,
        to_liquidator: 4133510,
        fee: 54445,
    }],
    post_collateral_usd_wad: 6649636000000000000000,
    post_weighted_usd_wad: 4654745200000000000000,
    post_debt_usd_wad: 4047604466356850000000,
    outcome: VectorOutcome::Open {
        hf_wad: 1150000015735139890,
    },
};

const V5: LiquidationVector = LiquidationVector {
    name: "V5 XLM+EURC->USDC blend",
    spoke: 1,
    open_prices: &[
        ("XLM", 250000000000000000),
        ("EURC", 1140000000000000000),
        ("USDC", 1000000000000000000),
    ],
    collateral: &[("XLM", 20000000000), ("EURC", 10000000000)],
    debt: &[("USDC", 11550000000)],
    prices: &[
        ("XLM", 150000000000000000),
        ("EURC", 1140000000000000000),
        ("USDC", 1000000000000000000),
    ],
    hf_wad: 982337662337662337,
    bonus_bps: 795,
    repay_usd_wad: 646698638000000000000,
    repaid: &[("USDC", 6466986380)],
    seized: &[
        SeizedLeg {
            asset: "XLM",
            gross: 9695988607,
            to_liquidator: 9610301038,
            fee: 85687569,
        },
        SeizedLeg {
            asset: "EURC",
            gross: 4847994303,
            to_liquidator: 4812291150,
            fee: 35703153,
        },
    ],
    post_collateral_usd_wad: 741888820353000000000,
    post_weighted_usd_wad: 584546566369920000000,
    post_debt_usd_wad: 508301362000000000000,
    outcome: VectorOutcome::Open {
        hf_wad: 1150000000137556192,
    },
};

const V6: LiquidationVector = LiquidationVector {
    name: "V6 dust promotion XLM->USDC",
    spoke: 1,
    open_prices: &[("XLM", 200000000000000000), ("USDC", 1000000000000000000)],
    collateral: &[("XLM", 500000000)],
    debt: &[("USDC", 75000000)],
    prices: &[("XLM", 190000000000000000), ("USDC", 1000000000000000000)],
    hf_wad: 988000000000000000,
    bonus_bps: 1087,
    repay_usd_wad: 7500000000000000000,
    repaid: &[("USDC", 75000000)],
    seized: &[SeizedLeg {
        asset: "XLM",
        gross: 437644736,
        to_liquidator: 432495789,
        fee: 5148947,
    }],
    post_collateral_usd_wad: 1184750016000000000,
    post_weighted_usd_wad: 924105012480000000,
    post_debt_usd_wad: 0,
    outcome: VectorOutcome::DebtFree,
};

const V7: LiquidationVector = LiquidationVector {
    name: "V7 acct-1 shape insolvent seize_all",
    spoke: 1,
    open_prices: &[
        ("USDT0", 1000000000000000000),
        ("USDC", 1000000000000000000),
        ("XLM", 200000000000000000),
    ],
    collateral: &[("USDT0", 220000000), ("USDC", 100000000)],
    debt: &[("XLM", 1000000000)],
    prices: &[
        ("USDT0", 1000000000000000000),
        ("USDC", 1000000000000000000),
        ("XLM", 330000000000000000),
    ],
    hf_wad: 769090909090909090,
    bonus_bps: 469,
    repay_usd_wad: 30566434206000000000,
    repaid: &[("XLM", 926255582)],
    seized: &[
        SeizedLeg {
            asset: "USDT0",
            gross: 220000000,
            to_liquidator: 219014424,
            fee: 985576,
        },
        SeizedLeg {
            asset: "USDC",
            gross: 100000000,
            to_liquidator: 99552011,
            fee: 447989,
        },
    ],
    post_collateral_usd_wad: 0,
    post_weighted_usd_wad: 0,
    post_debt_usd_wad: 2433565794000000000,
    outcome: VectorOutcome::BadDebt {
        socialized: &[("XLM", 73744418)],
    },
};

fn check(v: &LiquidationVector) {
    let mut m = MainnetSpoke::open_vector(v, ALICE);
    m.assert_transfer_vector(LIQUIDATOR, ALICE, v);
}

#[test]
fn redteam_spoke1_v1_xlm_usdc_target_restores_hf_1_15() {
    check(&V1);
}

#[test]
fn redteam_spoke1_v2a_xlm_usdc_capped_dust_promotes_full_close() {
    check(&V2A);
}

#[test]
fn redteam_spoke1_v2b_xlm_usdc_band_closes_at_cap() {
    check(&V2B);
}

#[test]
fn redteam_spoke1_v3_xlm_usdc_insolvent_seizes_all_and_cleans_bad_debt() {
    check(&V3);
}

#[test]
fn redteam_spoke1_v4_solvbtc_xlm_debt_rally_target() {
    check(&V4);
}

#[test]
fn redteam_spoke1_v5_xlm_eurc_blend_seizes_pro_rata() {
    check(&V5);
}

#[test]
fn redteam_spoke1_v6_small_account_dust_promotes_full_close() {
    check(&V6);
}

#[test]
fn redteam_spoke1_v7_two_stable_legs_insolvent_seize_all() {
    check(&V7);
}

fn v7_without_usdt0_cash() -> MainnetSpoke {
    let mut m = MainnetSpoke::build(V7.spoke);
    m.set_prices(V7.open_prices);
    m.set_pool_cash("USDT0", 0);
    m.open_position(ALICE, V7.collateral, V7.debt);
    m.open_position(BOB, &[("XLM", 10_000_000_000)], &[("USDT0", 120_000_000)]);
    m.set_prices(V7.prices);
    m
}

#[test]
fn redteam_spoke1_v7_transfer_reverts_when_usdt0_cash_is_short() {
    let mut m = v7_without_usdt0_cash();
    assert_eq!(m.pool_state("USDT0").cash, 100_000_000);
    let res = m.try_liquidate(LIQUIDATOR, ALICE, V7.debt, SeizeMode::Transfer);
    assert_contract_error(res, INSUFFICIENT_LIQUIDITY);
}

#[test]
fn redteam_spoke1_v7_credit_succeeds_when_usdt0_cash_is_short() {
    let mut m = v7_without_usdt0_cash();
    assert_eq!(m.health_factor_raw(ALICE), V7.hf_wad);
    let obs = m
        .observe_liquidation(LIQUIDATOR, ALICE, V7.debt, SeizeMode::Credit(0))
        .expect("credit liquidation");
    assert_ne!(obs.receiver_id, 0);
    assert_eq!(obs.bonus_bps, V7.bonus_bps);
    assert_eq!(obs.repaid_usd_wad, V7.repay_usd_wad);
    let (asset, repaid) = V7.repaid[0];
    assert_eq!(obs.liquidator_delta[asset], V7.debt[0].1 - repaid);
    for leg in V7.seized {
        let unit = m.share_unit(leg.asset);
        assert_eq!(
            obs.shares_burned[leg.asset],
            leg.gross * unit,
            "{}",
            leg.asset
        );
        assert_eq!(
            obs.receiver_shares[leg.asset] + obs.revenue_delta[leg.asset],
            leg.gross * unit,
            "{}",
            leg.asset
        );
        assert_eq!(obs.liquidator_delta[leg.asset], 0, "{}", leg.asset);
    }
    assert!(obs.debt_free);
    assert_eq!(
        obs.bad_debt_event,
        Some((V7.post_debt_usd_wad, V7.post_collateral_usd_wad))
    );
    assert_eq!(m.pool_state("USDT0").cash, 100_000_000);
    m.t.assert_spoke_usage_matches_positions();
}
