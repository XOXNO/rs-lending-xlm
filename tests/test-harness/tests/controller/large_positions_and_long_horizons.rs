//! GH-06. One billion whole tokens at 3, 7 and 18 decimals, mainnet rate
//! curves, years of accrual at several utilizations. Every path must clear,
//! the borrow index must track an `e^(r t)` reference within the Taylor bound,
//! accrued interest must be fully assigned, and the exit must pay back what
//! the book says. The last test drives the one cliff the domain has: the ray
//! value of a whale position overflows `i128` long before the index cap.
//!
//! Cells are chosen under that cliff. Utilization is value-based and debt
//! compounds faster than supply, so an untouched market drifts upward: on the
//! XLM curve a book left at 50 percent utilization crosses the optimal point
//! after about ten years and runs away (the reference puts it at x8650 after
//! twenty). Every cell asserts the reference projection stays under the
//! ceiling, so a bad cell fails with a message rather than a host trap.

use common::math::fp::Ray;
use common::validation::max_cap_for_decimals;
use controller::constants::{MAX_BORROW_INDEX_RAY, RAY};
use test_harness::{
    assert_contract_error, errors, hub_asset, usd, LendingTest, MarketParamsPreset, MarketPreset,
    ALICE, BOB, DEFAULT_ASSET_CONFIG, HARNESS_SPOKE,
};

const YEAR_SECS: u64 = 31_556_926;
const BILLION: i128 = 1_000_000_000;

/// `configs/mainnet/markets.json`, XLM.
fn xlm_curve() -> MarketParamsPreset {
    MarketParamsPreset {
        max_borrow_rate: RAY * 175 / 100,
        base_borrow_rate: RAY / 100,
        slope1: RAY * 4 / 100,
        slope2: RAY * 10 / 100,
        slope3: RAY * 150 / 100,
        mid_utilization: RAY * 50 / 100,
        optimal_utilization: RAY * 75 / 100,
        max_utilization: RAY,
        reserve_factor: 2000,
    }
}

/// `configs/mainnet/markets.json`, USDC.
fn usdc_curve() -> MarketParamsPreset {
    MarketParamsPreset {
        max_borrow_rate: RAY * 125 / 100,
        base_borrow_rate: RAY * 5 / 1000,
        slope1: RAY * 3 / 100,
        slope2: RAY * 95 / 1000,
        slope3: RAY,
        mid_utilization: RAY * 60 / 100,
        optimal_utilization: RAY * 85 / 100,
        max_utilization: RAY,
        reserve_factor: 1500,
    }
}

/// The market under test. Price is one dollar so token and dollar amounts
/// coincide in the assertions.
fn big(name: &'static str, decimals: u32, params: MarketParamsPreset) -> MarketPreset {
    MarketPreset {
        name,
        decimals,
        price_wad: usd(1),
        initial_liquidity: 0.0,
        config: DEFAULT_ASSET_CONFIG,
        params,
    }
}

/// Collateral for the borrower, priced at one dollar with a 75 percent LTV.
fn col() -> MarketPreset {
    MarketPreset {
        name: "COL",
        decimals: 7,
        price_wad: usd(1),
        initial_liquidity: 0.0,
        config: DEFAULT_ASSET_CONFIG,
        params: usdc_curve(),
    }
}

fn lift_caps(t: &LendingTest, asset: &str, decimals: u32) {
    let cap = max_cap_for_decimals(decimals);
    let cfg = t.get_asset_config(asset);
    t.edit_asset_in_spoke_caps(
        asset,
        HARNESS_SPOKE,
        true,
        true,
        cfg.loan_to_value,
        cfg.liquidation_threshold,
        cfg.liquidation_bonus,
        cap,
        cap,
    );
}

/// Relative shortfall of the eighth-order Taylor series against `e^x`.
fn taylor_tail(x: f64) -> f64 {
    let mut term = 1.0;
    let mut sum = 1.0;
    for k in 1..=8 {
        term *= x / k as f64;
        sum += term;
    }
    1.0 - sum / x.exp()
}

struct Book {
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
}

fn book(t: &LendingTest, asset: &str) -> Book {
    let key = hub_asset(t.resolve_asset(asset));
    let s = t.pool_client(asset).get_sync_data(&key).state;
    Book {
        supplied: s.supplied,
        borrowed: s.borrowed,
        revenue: s.revenue,
        supply_index: s.supply_index,
        borrow_index: s.borrow_index,
    }
}

/// Annual rate from the same piecewise curve, in floating point.
fn annual_rate(p: &MarketParamsPreset, util: f64) -> f64 {
    let ray = RAY as f64;
    let (base, s1, s2, s3) = (
        p.base_borrow_rate as f64 / ray,
        p.slope1 as f64 / ray,
        p.slope2 as f64 / ray,
        p.slope3 as f64 / ray,
    );
    let (mid, opt) = (
        p.mid_utilization as f64 / ray,
        p.optimal_utilization as f64 / ray,
    );
    let u = util.min(1.0);
    let rate = if u < mid {
        base + s1 * u / mid
    } else if u < opt {
        base + s1 + s2 * (u - mid) / (opt - mid)
    } else {
        base + s1 + s2 + s3 * (u - opt) / (1.0 - opt)
    };
    rate.min(p.max_borrow_rate as f64 / ray)
}

/// Floating-point replay of one-year chunks: utilization, rate, `e^(r t)`,
/// reward split, revenue shares. Returns the reference borrow index and the
/// summed per-chunk Taylor tail the contract is allowed to lose.
fn reference(p: &MarketParamsPreset, start: &Book, years: u32) -> (f64, f64) {
    let ray = RAY as f64;
    let mut supplied = start.supplied as f64;
    let borrowed = start.borrowed as f64;
    let mut bi = start.borrow_index as f64 / ray;
    let mut si = start.supply_index as f64 / ray;
    let mut allowed = 0.0;
    for _ in 0..years {
        let util = if supplied == 0.0 {
            0.0
        } else {
            (borrowed * bi) / (supplied * si)
        };
        let r = annual_rate(p, util);
        allowed += taylor_tail(r);
        let new_bi = bi * r.exp();
        let interest = borrowed * (new_bi - bi);
        let fee = interest * p.reserve_factor as f64 / 10_000.0;
        let rewards = interest - fee;
        let new_si = if supplied == 0.0 {
            si
        } else {
            (supplied * si + rewards) / supplied
        };
        supplied += fee / new_si;
        bi = new_bi;
        si = new_si;
    }
    (bi, allowed)
}

/// One cell of the matrix: supply a billion tokens, borrow `util_bps` of it,
/// accrue `years`, check the index, the conservation law and the exit.
fn run_cell(
    name: &'static str,
    decimals: u32,
    params: MarketParamsPreset,
    util_bps: i128,
    years: u32,
) {
    let unit = 10i128.pow(decimals);
    let mut t = LendingTest::new()
        .with_market(big(name, decimals, params.clone()))
        .with_market(col())
        .with_max_utilization_disabled_all_markets()
        .build();
    lift_caps(&t, name, decimals);
    lift_caps(&t, "COL", 7);

    let principal = BILLION * unit;
    t.supply_raw(BOB, name, principal);
    let debt = principal / 10_000 * util_bps;
    if debt > 0 {
        // Collateral worth twice the debt at a 75 percent LTV.
        t.supply_raw(ALICE, "COL", debt / unit * 10_000_000 * 2 + 10_000_000);
        t.borrow_raw(ALICE, name, debt);
    }
    let before = book(&t, name);

    for _ in 0..years {
        t.advance_and_sync_markets(YEAR_SECS, &[name, "COL"]);
    }
    let after = book(&t, name);

    // Borrow index against the reference, one-sided: the contract may only under-accrue.
    let (ref_bi, allowed) = reference(&params, &before, years);
    let projected_debt_whole = (debt / unit) as f64 * ref_bi;
    assert!(
        projected_debt_whole < 1.6e11,
        "{name} u={util_bps} y={years}: the reference projects {projected_debt_whole:e} whole tokens of debt, past the ray-value ceiling; pick a shorter horizon"
    );
    let got_bi = after.borrow_index as f64 / RAY as f64;
    assert!(
        got_bi <= ref_bi * (1.0 + 1e-9),
        "{name} u={util_bps} y={years}: contract over-accrued {got_bi} > {ref_bi}"
    );
    let shortfall = (ref_bi - got_bi) / ref_bi;
    // Twice the tail: the truncated interest also feeds back through the
    // revenue shares into the next chunk's utilization, and on the steep
    // segment the curve amplifies that into the next chunk's rate. Measured
    // excess over one tail is a few percent of the tail.
    assert!(
        shortfall <= 2.0 * allowed + 1e-9,
        "{name} u={util_bps} y={years}: shortfall {shortfall:e} exceeds twice the Taylor bound {allowed:e}"
    );
    std::println!(
        "{name} d={decimals} u={util_bps}bps y={years}: index x{got_bi:.4}, reference x{ref_bi:.4}, shortfall {shortfall:e}"
    );

    // Conservation: interest == supplier gain + revenue gain, to a handful of raw ray units per chunk.
    let env = &t.env;
    let interest = Ray::from(after.borrowed)
        .mul(env, Ray::from(after.borrow_index))
        .checked_sub(
            env,
            Ray::from(before.borrowed).mul(env, Ray::from(before.borrow_index)),
        );
    let supplier_gain = Ray::from(before.supplied)
        .mul(env, Ray::from(after.supply_index))
        .checked_sub(
            env,
            Ray::from(before.supplied).mul(env, Ray::from(before.supply_index)),
        );
    let minted = after.supplied - before.supplied;
    assert_eq!(
        minted,
        after.revenue - before.revenue,
        "every minted share is a revenue share"
    );
    let revenue_gain = Ray::from(minted).mul(env, Ray::from(after.supply_index));
    let assigned = supplier_gain.checked_add(env, revenue_gain);
    let slack = (after.supply_index / RAY + 4) * years as i128;
    assert!(
        interest.raw() >= assigned.raw() && interest.raw() - assigned.raw() <= slack,
        "{name} u={util_bps} y={years}: interest {} vs assigned {} (slack {slack})",
        interest.raw(),
        assigned.raw()
    );

    // Exit: repay all with one unit of overpayment, then the supplier withdraws all.
    if debt > 0 {
        let owed = t.borrow_balance_raw(ALICE, name) + 1;
        t.repay_raw(ALICE, name, owed);
        assert_eq!(t.borrow_balance_raw(ALICE, name), 0);
    }
    let claimed = t.supply_balance_raw(BOB, name);
    t.withdraw_all(BOB, name);
    let paid = t.token_balance_raw(BOB, name);
    assert!(paid >= principal, "{name}: exit below principal");
    assert!(paid <= claimed, "{name}: exit above the book value");
}

#[test]
fn one_billion_at_seven_decimals_accrues_and_exits_on_the_xlm_curve() {
    for (util, years) in [(0, 20), (5_000, 10), (8_000, 3), (9_500, 2), (9_800, 1)] {
        run_cell("BIG7", 7, xlm_curve(), util, years);
    }
}

#[test]
fn one_billion_at_seven_decimals_accrues_and_exits_on_the_usdc_curve() {
    for (util, years) in [(5_000, 20), (8_000, 10), (9_500, 3), (9_800, 2)] {
        run_cell("BIG7", 7, usdc_curve(), util, years);
    }
}

#[test]
fn one_billion_at_eighteen_decimals_accrues_and_exits() {
    for (util, years) in [(0, 20), (5_000, 10), (8_000, 3), (9_500, 2)] {
        run_cell("BIG18", 18, xlm_curve(), util, years);
    }
}

#[test]
fn one_billion_at_three_decimals_accrues_and_exits() {
    for (util, years) in [(5_000, 20), (9_500, 2)] {
        run_cell("BIG3", 3, usdc_curve(), util, years);
    }
}

/// The cliff. A billion whole tokens is `1e36` raw ray; the value ceiling is
/// `i128::MAX`, 170 times that. At the XLM curve's steep segment the index
/// grows past 170x in a few years, and the next accrual panics inside
/// `scaled_to_original`. Every verb accrues first, so the market freezes:
/// no repay, no withdraw, no liquidation. The index cap never engages.
#[test]
fn a_whale_market_at_sustained_high_utilization_hits_the_ray_value_ceiling_before_the_index_cap() {
    let mut t = LendingTest::new()
        .with_market(big("BIG18", 18, xlm_curve()))
        .with_market(col())
        .with_max_utilization_disabled_all_markets()
        .build();
    lift_caps(&t, "BIG18", 18);
    lift_caps(&t, "COL", 7);
    let principal = BILLION * 10i128.pow(18);
    t.supply_raw(BOB, "BIG18", principal);
    let debt = principal / 100 * 98;
    t.supply_raw(ALICE, "COL", BILLION * 10_000_000 * 3);
    t.borrow_raw(ALICE, "BIG18", debt);

    let mut years = 0u32;
    let failure = loop {
        years += 1;
        assert!(
            years <= 40,
            "no cliff within 40 years; the bound in numeric-bounds.md is wrong"
        );
        t.advance_time(YEAR_SECS);
        if let Err(e) = t.try_update_indexes_for(&["BIG18"]) {
            break e;
        }
    };
    let failed: Result<(), soroban_sdk::Error> = Err(failure);
    assert_contract_error(failed, errors::MATH_OVERFLOW);
    let last = book(&t, "BIG18");
    assert!(
        last.borrow_index < MAX_BORROW_INDEX_RAY,
        "the index cap did not engage before the value overflow"
    );
    // The market is frozen: exits and repayments accrue first and hit the same panic.
    assert_contract_error(t.try_withdraw_raw(BOB, "BIG18", 1), errors::MATH_OVERFLOW);
    assert_contract_error(t.try_repay(ALICE, "BIG18", 1.0), errors::MATH_OVERFLOW);
    std::println!(
        "ray-value cliff reached after {years} years at 98 percent utilization on the XLM curve; last index x{:.1}",
        last.borrow_index as f64 / RAY as f64
    );
}
