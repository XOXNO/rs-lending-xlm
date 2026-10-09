use common::errors::OracleError;
use common::types::SeizeMode;
use controller::constants::WAD;
use test_harness::errors::codes::HEALTH_FACTOR_TOO_HIGH;
use test_harness::mainnet::{mainnet_market, MainnetSpoke};
use test_harness::{assert_contract_error, ALICE, LIQUIDATOR};

const SANITY_BOUND_VIOLATED: u32 = OracleError::SanityBoundViolated as u32;
const MOCK_PRICE_STEP: i128 = 10_000;
const COLLATERAL_USD: i128 = 10_000 * WAD;

#[derive(Clone, Copy)]
enum Edge {
    Floor,
    Ceiling,
}

struct DarkCase {
    spoke: u32,
    collateral: &'static str,
    debt: &'static str,
    mover: &'static str,
    edge: Edge,
}

fn open_max_ltv(case: &DarkCase) -> MainnetSpoke {
    let mut m = MainnetSpoke::build(case.spoke);
    let cap = m.listing(case.collateral).supply_cap / 2;
    let amount = m.usd_to_raw(case.collateral, COLLATERAL_USD).min(cap);
    m.open_max_ltv(ALICE, case.collateral, amount, case.debt);
    m
}

fn band_edge(asset: &str, edge: Edge) -> (i128, i128) {
    let market = mainnet_market(asset);
    match edge {
        Edge::Floor => (
            market.min_sanity_price_wad,
            market.min_sanity_price_wad - MOCK_PRICE_STEP,
        ),
        Edge::Ceiling => (
            market.max_sanity_price_wad,
            market.max_sanity_price_wad + MOCK_PRICE_STEP,
        ),
    }
}

fn full_debt(m: &MainnetSpoke, debt: &'static str) -> [(&'static str, i128); 1] {
    [(debt, m.debt_raw(ALICE, debt))]
}

fn assert_dark_before_liquidatable(case: &DarkCase) {
    let ctx = format!(
        "spoke {} {} against {} ({} at its band edge)",
        case.spoke, case.collateral, case.debt, case.mover
    );
    let mut m = open_max_ltv(case);
    let payments = full_debt(&m, case.debt);
    let (inside, outside) = band_edge(case.mover, case.edge);

    m.set_price(case.mover, inside);
    assert_eq!(m.price(case.mover), inside, "{ctx}: edge price is exact");
    let hf = m.health_factor_raw(ALICE);
    assert!(hf >= WAD, "{ctx}: HF {hf} at the band edge is below 1");
    let res = m.try_liquidate(LIQUIDATOR, ALICE, &payments, SeizeMode::Transfer);
    assert_contract_error(res, HEALTH_FACTOR_TOO_HIGH);

    m.set_price(case.mover, outside);
    assert_eq!(m.t.try_health_factor_raw(ALICE), None, "{ctx}: HF view");
    let res = m.try_liquidate(LIQUIDATOR, ALICE, &payments, SeizeMode::Transfer);
    assert_contract_error(res, SANITY_BOUND_VIOLATED);
    let res = m.try_liquidate(LIQUIDATOR, ALICE, &payments, SeizeMode::Credit(0));
    assert_contract_error(res, SANITY_BOUND_VIOLATED);
}

#[test]
fn redteam_dark_spoke1_usdc_collateral_against_usd_debt() {
    assert_dark_before_liquidatable(&DarkCase {
        spoke: 1,
        collateral: "USDC",
        debt: "PYUSD",
        mover: "USDC",
        edge: Edge::Floor,
    });
}

#[test]
fn redteam_dark_spoke1_eurc_collateral_against_usd_debt() {
    assert_dark_before_liquidatable(&DarkCase {
        spoke: 1,
        collateral: "EURC",
        debt: "USDC",
        mover: "EURC",
        edge: Edge::Floor,
    });
}

#[test]
fn redteam_dark_spoke1_pyusd_collateral_against_usd_debt() {
    assert_dark_before_liquidatable(&DarkCase {
        spoke: 1,
        collateral: "PYUSD",
        debt: "USDC",
        mover: "PYUSD",
        edge: Edge::Floor,
    });
}

#[test]
fn redteam_dark_spoke1_usdt0_collateral_against_usd_debt() {
    assert_dark_before_liquidatable(&DarkCase {
        spoke: 1,
        collateral: "USDT0",
        debt: "USDC",
        mover: "USDT0",
        edge: Edge::Floor,
    });
}

#[test]
fn redteam_dark_spoke5_ustryusdc_lp_quoted_directly() {
    assert_dark_before_liquidatable(&DarkCase {
        spoke: 5,
        collateral: "USTRYUSDC_LP",
        debt: "XLM",
        mover: "USTRYUSDC_LP",
        edge: Edge::Floor,
    });
}

#[test]
fn redteam_dark_spoke5_usdyusdc_lp_quoted_directly() {
    assert_dark_before_liquidatable(&DarkCase {
        spoke: 5,
        collateral: "USDYUSDC_LP",
        debt: "XLM",
        mover: "USDYUSDC_LP",
        edge: Edge::Floor,
    });
}

#[test]
fn redteam_dark_spoke5_pyusdusdc_lp_quoted_directly() {
    assert_dark_before_liquidatable(&DarkCase {
        spoke: 5,
        collateral: "PYUSDUSDC_LP",
        debt: "XLM",
        mover: "PYUSDUSDC_LP",
        edge: Edge::Floor,
    });
}

#[test]
fn redteam_dark_spoke6_usdyusdc_lp_quoted_directly() {
    assert_dark_before_liquidatable(&DarkCase {
        spoke: 6,
        collateral: "USDYUSDC_LP",
        debt: "USDC",
        mover: "USDYUSDC_LP",
        edge: Edge::Floor,
    });
}

#[test]
fn redteam_dark_spoke5_lp_against_usdc_debt_rally() {
    assert_dark_before_liquidatable(&DarkCase {
        spoke: 5,
        collateral: "XLMUSDC_LP",
        debt: "USDC",
        mover: "USDC",
        edge: Edge::Ceiling,
    });
}

#[test]
fn redteam_dark_spoke5_cetesusdc_lp_is_liquidatable_at_its_floor() {
    let case = DarkCase {
        spoke: 5,
        collateral: "CETESUSDC_LP",
        debt: "XLM",
        mover: "CETESUSDC_LP",
        edge: Edge::Floor,
    };
    let mut m = open_max_ltv(&case);
    let (inside, outside) = band_edge(case.mover, case.edge);
    m.set_price(case.mover, inside);
    assert!(m.health_factor_raw(ALICE) < WAD);
    m.set_price(case.mover, outside);
    assert_eq!(m.t.try_health_factor_raw(ALICE), None);
}

fn usdy_window(spoke: u32) {
    let case = DarkCase {
        spoke,
        collateral: "USDY",
        debt: "USDC",
        mover: "USDY",
        edge: Edge::Floor,
    };
    let mut m = open_max_ltv(&case);
    let payments = full_debt(&m, case.debt);
    let (floor, outside) = band_edge("USDY", Edge::Floor);
    let mut hf = Vec::new();
    for price in [
        floor + 300 * WAD / 1_000_000,
        floor + 200 * WAD / 1_000_000,
        floor,
    ] {
        m.set_price("USDY", price);
        hf.push(m.health_factor_raw(ALICE));
    }
    assert_eq!(
        hf,
        [
            1_000_064_456_847_670_819,
            999_971_884_001_531_074,
            999_786_738_309_251_583
        ]
    );
    let estimate = m.estimate(ALICE, &payments, SeizeMode::Transfer);
    assert!(estimate.max_payment_wad > 0);
    m.set_price("USDY", outside);
    assert_eq!(m.t.try_health_factor_raw(ALICE), None);
    let res = m.try_liquidate(LIQUIDATOR, ALICE, &payments, SeizeMode::Transfer);
    assert_contract_error(res, SANITY_BOUND_VIOLATED);
}

#[test]
fn redteam_dark_spoke4_usdy_max_ltv_liquidatable_only_within_0_03_percent_of_floor() {
    usdy_window(4);
}

#[test]
fn redteam_dark_spoke6_usdy_max_ltv_liquidatable_only_within_0_03_percent_of_floor() {
    usdy_window(6);
}
