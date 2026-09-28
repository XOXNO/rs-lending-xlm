use common::types::{LiquidationEstimate, SeizeMode};
use controller::constants::WAD;
use test_harness::mainnet::{mainnet_onchain_spoke_ids, MainnetSpoke};
use test_harness::reference::{ExactBook, ExactPlan};
use test_harness::{ALICE, LIQUIDATOR};

const COLLATERAL_USD: i128 = 2_000 * WAD;
const MAX_STEPS: usize = 200;

struct Case {
    spoke: u32,
    collateral: &'static str,
    debt: &'static str,
    mover: &'static str,
    step_bps: i128,
}

const CASES: [Case; 8] = [
    Case {
        spoke: 1,
        collateral: "XLM",
        debt: "USDC",
        mover: "XLM",
        step_bps: -200,
    },
    Case {
        spoke: 2,
        collateral: "USTRY",
        debt: "XLM",
        mover: "XLM",
        step_bps: 300,
    },
    Case {
        spoke: 3,
        collateral: "DEJTRSY",
        debt: "XLM",
        mover: "XLM",
        step_bps: 300,
    },
    Case {
        spoke: 4,
        collateral: "EURC",
        debt: "USDC",
        mover: "EURC",
        step_bps: -50,
    },
    Case {
        spoke: 5,
        collateral: "XLMUSDC_LP",
        debt: "XLM",
        mover: "XLM",
        step_bps: 300,
    },
    Case {
        spoke: 6,
        collateral: "USDY",
        debt: "XLM",
        mover: "XLM",
        step_bps: 300,
    },
    Case {
        spoke: 7,
        collateral: "XAUM",
        debt: "XLM",
        mover: "XLM",
        step_bps: 300,
    },
    Case {
        spoke: 8,
        collateral: "AQUA",
        debt: "XLM",
        mover: "XLM",
        step_bps: 300,
    },
];

fn assert_estimate_matches(
    book: &ExactBook,
    plan: &ExactPlan,
    estimate: &LiquidationEstimate,
    credit: bool,
    context: &str,
) {
    assert_eq!(estimate.bonus_rate_bps, plan.bonus_bps, "{context}: bonus");
    assert_eq!(
        estimate.max_payment_wad, plan.repay_usd,
        "{context}: repayment"
    );
    let seized: Vec<_> = plan
        .seized
        .iter()
        .map(|s| {
            let asset = book.key_of(s.asset_id).asset.clone();
            if credit {
                (asset, s.scaled_amount, s.credit_fee_scaled)
            } else {
                (asset, s.amount, s.protocol_fee)
            }
        })
        .collect();
    let actual: Vec<_> = estimate
        .seized_collaterals
        .iter()
        .zip(estimate.protocol_fees.iter())
        .map(|(s, f)| (s.asset, s.amount, f.amount))
        .collect();
    assert_eq!(actual, seized, "{context}: seizure and fee");
    let refunds: Vec<_> = plan
        .refunds
        .iter()
        .map(|(id, amount)| (book.key_of(*id).asset.clone(), *amount))
        .collect();
    let actual_refunds: Vec<_> = estimate
        .refunds
        .iter()
        .map(|r| (r.asset, r.amount))
        .collect();
    assert_eq!(actual_refunds, refunds, "{context}: refunds");
}

fn check_checkpoint(m: &MainnetSpoke, case: &Case, step: usize) -> bool {
    let debt = m.debt_raw(ALICE, case.debt);
    let mut insolvent = false;
    for offer in [debt, debt / 3] {
        let payments = [(case.debt, offer)];
        let (book, plan) = m.reference_plan(ALICE, &payments);
        let context = format!("spoke {} step {step} offer {offer}", case.spoke);
        assert_eq!(
            m.health_factor_raw(ALICE),
            plan.totals.health_factor,
            "{context}: health factor"
        );
        for (mode, credit) in [(SeizeMode::Transfer, false), (SeizeMode::Credit(0), true)] {
            let estimate = m.estimate(ALICE, &payments, mode);
            assert_estimate_matches(&book, &plan, &estimate, credit, &context);
        }
        insolvent = plan.totals.total_collateral < plan.totals.total_debt;
    }
    insolvent
}

fn run(case: &Case) {
    let mut m = MainnetSpoke::build(case.spoke);
    let cap = m.listing(case.collateral).supply_cap / 2;
    let collateral = m.usd_to_raw(case.collateral, COLLATERAL_USD).min(cap);
    m.open_max_ltv(ALICE, case.collateral, collateral, case.debt);

    let mut checkpoints = 0;
    for step in 0..MAX_STEPS {
        let next = m.price(case.mover) * (10_000 + case.step_bps) / 10_000;
        if !m.price_in_band(case.mover, next) {
            break;
        }
        m.move_price_bps(case.mover, case.step_bps);
        if m.health_factor_raw(ALICE) >= WAD {
            continue;
        }
        checkpoints += 1;
        if check_checkpoint(&m, case, step) {
            break;
        }
    }
    assert!(
        checkpoints > 0,
        "spoke {}: {} never made the account liquidatable inside its sanity band",
        case.spoke,
        case.mover
    );

    let offer = m.debt_raw(ALICE, case.debt);
    let payments = [(case.debt, offer)];
    if case.spoke.is_multiple_of(2) {
        m.liquidate_credit(LIQUIDATOR, ALICE, &payments, 0);
    } else {
        m.liquidate_transfer(LIQUIDATOR, ALICE, &payments);
    }
}

#[test]
fn redteam_smoke_covers_every_onchain_spoke() {
    let spokes: Vec<u32> = CASES.iter().map(|c| c.spoke).collect();
    assert_eq!(spokes, mainnet_onchain_spoke_ids());
}

#[test]
fn redteam_smoke_estimate_matches_exact_reference_on_every_mainnet_spoke() {
    for case in &CASES {
        run(case);
    }
}
