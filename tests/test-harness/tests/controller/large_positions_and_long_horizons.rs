//! GH-06. One billion whole tokens at 3, 7 and 18 decimals, steep stress rate
//! curves, years of accrual at several utilizations. Every path must clear,
//! the borrow index must track an `e^(r t)` reference within the Taylor bound,
//! accrued interest must be fully assigned, and the exit must pay back what
//! the book says. The last test drives the market value ceiling: a whale
//! market's total value reaches `MAX_MARKET_VALUE_RAY` long before the index
//! cap, and accrual stops its interest there instead of overflowing.
//!
//! Cells are chosen under that ceiling. Utilization is value-based and debt
//! compounds faster than supply, so an untouched market drifts upward: on the
//! XLM curve a book left at 50 percent utilization crosses the optimal point
//! after about ten years and runs away (the reference puts it at x8650 after
//! twenty). Every cell asserts the reference projection stays under the
//! ceiling, so a bad cell fails with a message rather than a capped index that
//! no longer matches the reference.

use crate::shared::count_topic;
use common::math::fp::Ray;
use common::validation::max_cap_for_decimals;
use controller::constants::{MAX_BORROW_INDEX_RAY, MAX_MARKET_VALUE_RAY, RAY};
use controller::types::InterestRateModel;
use soroban_sdk::testutils::Events;
use test_harness::{
    hub_asset, usd, LendingTest, MarketParamsPreset, MarketPreset, ALICE, BOB,
    DEFAULT_ASSET_CONFIG, HARNESS_SPOKE,
};

const YEAR_SECS: u64 = 31_556_926;
const BILLION: i128 = 1_000_000_000;
/// Years the whale market accrues; the ceiling engages within the first few.
const CEILING_HORIZON_YEARS: u32 = 40;
/// Years the other markets accrue while the whale market sits at the ceiling.
const CONTAGION_HORIZON_YEARS: u32 = 6;

/// Steep XLM stress curve: 175 percent max borrow rate, optimal at 75 percent.
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

/// Steep USDC stress curve: 125 percent max borrow rate, optimal at 85 percent.
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

/// The ceiling. A billion whole tokens is `1e36` raw ray; the market value
/// ceiling sits about 170 times above. At the XLM curve's steep segment the
/// debt grows past it within a few years. Accrual then holds the market's
/// interest at the ceiling and emits `MarketValueCeilingEvent`, and every verb,
/// including the owner's rate-model change, keeps working. The index cap
/// never engages.
#[test]
fn a_whale_market_at_sustained_high_utilization_stops_accruing_at_the_value_ceiling() {
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

    let mut capped_in = None;
    for year in 1..=CEILING_HORIZON_YEARS {
        t.advance_time(YEAR_SECS);
        t.update_indexes_for(&["BIG18"]);
        if capped_in.is_none() && count_topic(&t.env.events().all(), "market", "value_ceiling") > 0
        {
            capped_in = Some(year);
        }
    }
    let capped_in = capped_in.expect("the value ceiling never engaged");

    let last = book(&t, "BIG18");
    let env = &t.env;
    let debt_value = Ray::from(last.borrowed).mul_floor(env, Ray::from(last.borrow_index));
    let supply_value = Ray::from(last.supplied).mul_floor(env, Ray::from(last.supply_index));
    assert!(debt_value.raw() <= MAX_MARKET_VALUE_RAY);
    assert!(supply_value.raw() <= MAX_MARKET_VALUE_RAY);
    assert!(
        last.borrow_index < MAX_BORROW_INDEX_RAY,
        "the value ceiling engages before the index cap"
    );

    // Exits, repayment and the owner's rate-model change all accrue first.
    t.withdraw_raw(BOB, "BIG18", 1);
    t.repay(ALICE, "BIG18", 1.0);
    let p = xlm_curve();
    let model = InterestRateModel {
        max_borrow_rate: p.max_borrow_rate,
        base_borrow_rate: p.base_borrow_rate,
        slope1: p.slope1,
        slope2: p.slope2,
        slope3: p.slope3,
        mid_utilization: p.mid_utilization,
        optimal_utilization: p.optimal_utilization,
        max_utilization: p.max_utilization,
        reserve_factor: p.reserve_factor,
        is_flashloanable: false,
        flashloan_fee: 0,
    };
    t.advance_time(YEAR_SECS);
    t.ctrl_client()
        .upgrade_liquidity_pool_params(&hub_asset(t.resolve_asset("BIG18")), &model);
    std::println!(
        "value ceiling engaged in year {capped_in} at 98 percent utilization on the XLM curve; index x{:.1}",
        last.borrow_index as f64 / RAY as f64
    );
}

/// A seven-decimal market at `price_wad` on `params`, with no seed.
fn priced(name: &'static str, price_wad: i128, params: MarketParamsPreset) -> MarketPreset {
    MarketPreset {
        price_wad,
        ..big(name, 7, params)
    }
}

/// A market held at the value ceiling still projects its index, so accounts
/// that hold a small leg in it stay valuable, liquidatable and cleanable on
/// their other markets. The controller values an account through one
/// `get_bulk_indexes` call over every market it touches.
#[test]
fn a_market_at_the_value_ceiling_does_not_block_other_markets_liquidation_and_cleanup() {
    let mut t = LendingTest::new()
        .with_market(big("BIG18", 18, xlm_curve()))
        .with_market(col())
        .with_market(priced("USDC", usd(1), usdc_curve()))
        .with_market(priced("ETH", usd(2_000), usdc_curve()))
        .with_max_utilization_disabled_all_markets()
        .build();
    lift_caps(&t, "BIG18", 18);
    lift_caps(&t, "ETH", 7);
    let principal = BILLION * 10i128.pow(18);
    t.supply_raw(BOB, "BIG18", principal);
    t.supply("lp", "USDC", 1_000_000.0);

    // Carol and Frank borrow USDC against COL and hold a small BIG18 debt leg.
    t.supply("carol", "COL", 1_000.0);
    t.borrow("carol", "USDC", 600.0);
    t.borrow("carol", "BIG18", 0.01);
    t.supply("frank", "COL", 10.0);
    t.borrow("frank", "USDC", 5.0);
    t.borrow("frank", "BIG18", 0.01);

    // The whale borrows 98 percent of BIG18 against ETH and stays healthy.
    t.supply_raw(ALICE, "ETH", BILLION * 10_000_000);
    t.borrow_raw(ALICE, "BIG18", principal / 100 * 98 - 10i128.pow(18));

    // A keeper syncs the other markets; BIG18 is only ever projected.
    for _ in 0..CONTAGION_HORIZON_YEARS {
        t.advance_time(YEAR_SECS);
        t.update_indexes_for(&["COL", "USDC", "ETH"]);
    }

    let big = hub_asset(t.resolve_asset("BIG18"));
    let markets = soroban_sdk::vec![
        &t.env,
        hub_asset(t.resolve_asset("COL")),
        hub_asset(t.resolve_asset("USDC")),
        big.clone(),
    ];
    let indexes = t.pool_client("COL").get_bulk_indexes(&markets);
    let big_index = indexes.get(2).unwrap().borrow_index;
    let borrowed = t.pool_client("BIG18").get_sync_data(&big).state.borrowed;
    let debt_value = Ray::from(borrowed)
        .mul_floor(&t.env, Ray::from(big_index))
        .raw();
    assert!(
        debt_value <= MAX_MARKET_VALUE_RAY && debt_value > MAX_MARKET_VALUE_RAY / 100 * 99,
        "BIG18's projected debt sits at the value ceiling: {debt_value}"
    );

    t.set_price("COL", usd(7) / 10);
    assert!(t.can_be_liquidated("carol"));
    let carol_debt = t.borrow_balance_raw("carol", "USDC");
    t.liquidate("liq", "carol", "USDC", 100.0);
    assert!(t.borrow_balance_raw("carol", "USDC") < carol_debt);

    t.set_price("COL", usd(1) / 10);
    let frank = t.resolve_account_id("frank");
    t.clean_bad_debt_by_id(frank);
    assert!(!t.ctrl_client().account_exists(&frank));
}
