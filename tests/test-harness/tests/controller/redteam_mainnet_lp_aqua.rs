use common::errors::OracleError;
use common::types::SeizeMode;
use test_harness::mainnet::{
    mainnet_market, LiquidationVector, MainnetSpoke, SeizedLeg, VectorOutcome,
};
use test_harness::{assert_contract_error, ALICE, LIQUIDATOR};

const V1: LiquidationVector = LiquidationVector {
    name: "AQUA/USDC s8",
    spoke: 8,
    open_prices: &[("AQUA", 362206653686974), ("USDC", 999990697386260000)],
    collateral: &[("AQUA", 10000000000000)],
    debt: &[("USDC", 1992155126)],
    prices: &[("AQUA", 325985988318276), ("USDC", 999990697386260000)],
    hf_wad: 981818182546326086,
    bonus_bps: 1381,
    repay_usd_wad: 71721786393717674128,
    repaid: &[("USDC", 717224536)],
    seized: &[SeizedLeg {
        asset: "AQUA",
        gross: 2503989987970,
        to_liquidator: 2467529112757,
        fee: 36460875213,
    }],
    post_collateral_usd_wad: 244359423215325453937,
    post_weighted_usd_wad: 146615653929195272361,
    post_debt_usd_wad: 127491872981317591970,
    outcome: VectorOutcome::Open {
        hf_wad: 1150000000005333999,
    },
};

const V2: LiquidationVector = LiquidationVector {
    name: "AQUA/USDC s8",
    spoke: 8,
    open_prices: &[("AQUA", 362206653686974), ("USDC", 999990697386260000)],
    collateral: &[("AQUA", 10000000000000)],
    debt: &[("USDC", 1992155126)],
    prices: &[("AQUA", 217323992212184), ("USDC", 999990697386260000)],
    hf_wad: 654545455040923529,
    bonus_bps: 909,
    repay_usd_wad: 199213659375035266097,
    repaid: &[("USDC", 1992155126)],
    seized: &[SeizedLeg {
        asset: "AQUA",
        gross: 9999916659097,
        to_liquidator: 9899926659173,
        fee: 99989999924,
    }],
    post_collateral_usd_wad: 1811197775434637,
    post_weighted_usd_wad: 1086718665260781,
    post_debt_usd_wad: 0,
    outcome: VectorOutcome::DebtFree,
};

const V3: LiquidationVector = LiquidationVector {
    name: "AQUA/XLM s8",
    spoke: 8,
    open_prices: &[("AQUA", 362206653686974), ("XLM", 212414871690620000)],
    collateral: &[("AQUA", 10000000000000)],
    debt: &[("XLM", 9378517517)],
    prices: &[("AQUA", 195591592990965), ("XLM", 212414871690620000)],
    hf_wad: 589090909163869575,
    bonus_bps: 1000,
    repay_usd_wad: 177810539081212907770,
    repaid: &[("XLM", 8370908198)],
    seized: &[SeizedLeg {
        asset: "AQUA",
        gross: 10000000000000,
        to_liquidator: 9890909090906,
        fee: 109090909094,
    }],
    post_collateral_usd_wad: 0,
    post_weighted_usd_wad: 0,
    post_debt_usd_wad: 21403120420965799689,
    outcome: VectorOutcome::BadDebt {
        socialized: &[("XLM", 1007609319)],
    },
};

const V4: LiquidationVector = LiquidationVector {
    name: "AQUAUSDC_LP/XLM s5",
    spoke: 5,
    open_prices: &[
        ("AQUAUSDC_LP", 41198623400785847),
        ("XLM", 212414871690620000),
    ],
    collateral: &[("AQUAUSDC_LP", 100000000000)],
    debt: &[("XLM", 9697678667)],
    prices: &[
        ("AQUAUSDC_LP", 32958898720628677),
        ("XLM", 212414871690620000),
    ],
    hf_wad: 960000000135654490,
    bonus_bps: 1431,
    repay_usd_wad: 84325186810764738038,
    repaid: &[("XLM", 3969834416)],
    seized: &[SeizedLeg {
        asset: "AQUAUSDC_LP",
        gross: 29246159545,
        to_liquidator: 28880038871,
        fee: 366120674,
    }],
    post_collateral_usd_wad: 233196866165125109868,
    post_weighted_usd_wad: 139918119699075065920,
    post_debt_usd_wad: 121667930164002041763,
    outcome: VectorOutcome::Open {
        hf_wad: 1150000000086076239,
    },
};

const V5: LiquidationVector = LiquidationVector {
    name: "XAUMUSDC_LP/USDC s7",
    spoke: 7,
    open_prices: &[
        ("XAUMUSDC_LP", 12949271541079476497),
        ("USDC", 999990697386260000),
    ],
    collateral: &[("XAUMUSDC_LP", 1000000000)],
    debt: &[("USDC", 6474696001)],
    prices: &[
        ("XAUMUSDC_LP", 9064490078755633547),
        ("USDC", 999990697386260000),
    ],
    hf_wad: 840000000147344540,
    bonus_bps: 1567,
    repay_usd_wad: 440180948228569915448,
    repaid: &[("USDC", 4401850431)],
    seized: &[SeizedLeg {
        asset: "XAUMUSDC_LP",
        gross: 561705400,
        to_liquidator: 554095888,
        fee: 7609512,
    }],
    post_collateral_usd_wad: 397291705327216734860,
    post_weighted_usd_wad: 238375023196330040915,
    post_debt_usd_wad: 207282628711831961987,
    outcome: VectorOutcome::Open {
        hf_wad: 1150000000857395941,
    },
};

const V6: LiquidationVector = LiquidationVector {
    name: "USDY/EURC s6",
    spoke: 6,
    open_prices: &[("USDY", 1147744770000000000), ("EURC", 1137204846581870000)],
    collateral: &[("USDY", 10000000000)],
    debt: &[("EURC", 8074146172)],
    prices: &[("USDY", 1080027828570000000), ("EURC", 1137204846581870000)],
    hf_wad: 999812500129702585,
    bonus_bps: 576,
    repay_usd_wad: 549320960798441712990,
    repaid: &[("EURC", 4830448643)],
    seized: &[SeizedLeg {
        asset: "USDY",
        gross: 5379137766,
        to_liquidator: 5349841403,
        fee: 29296363,
    }],
    post_collateral_usd_wad: 499065980470813922538,
    post_weighted_usd_wad: 424206083400191834157,
    post_debt_usd_wad: 368874855082443581520,
    outcome: VectorOutcome::Open {
        hf_wad: 1150000000150136867,
    },
};

fn check(v: &LiquidationVector) {
    let mut m = MainnetSpoke::open_vector(v, ALICE);
    m.assert_transfer_vector(LIQUIDATOR, ALICE, v);
}

#[test]
fn redteam_lp_aqua_v1_aqua_usdc_target() {
    check(&V1);
}

#[test]
fn redteam_lp_aqua_v2_aqua_usdc_band_closes_debt() {
    check(&V2);
}

#[test]
fn redteam_lp_aqua_v3_aqua_xlm_insolvent_cleans_bad_debt() {
    check(&V3);
}

#[test]
fn redteam_lp_aqua_v4_aquausdc_lp_xlm_target() {
    check(&V4);
}

#[test]
fn redteam_lp_aqua_v5_xaumusdc_lp_usdc_target() {
    check(&V5);
}

#[test]
fn redteam_lp_aqua_v6_usdy_eurc_just_above_band_floor() {
    check(&V6);
}

#[test]
fn redteam_lp_aqua_v6_one_step_lower_usdy_is_unpriceable() {
    let mut m = MainnetSpoke::open_vector(&V6, ALICE);
    let price = mainnet_market("USDY").price_wad * 9_409 / 10_000;
    assert!(price < mainnet_market("USDY").min_sanity_price_wad);
    m.set_price("USDY", price);
    let res = m.try_liquidate(LIQUIDATOR, ALICE, V6.debt, SeizeMode::Transfer);
    assert_contract_error(res, OracleError::SanityBoundViolated as u32);
}
