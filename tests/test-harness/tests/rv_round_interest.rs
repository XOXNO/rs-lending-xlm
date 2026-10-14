//! Rounding audit, lens R-7: interest accrual, allocation and revenue.
//!
//! The model runs the production step (`common::rates::accrue_step`, the one
//! implementation behind `pool::interest::global_sync` and
//! `simulate_update_indexes`) over a caller-chosen partition of a span and
//! re-derives every intermediate the step rounds (`common/src/rates/index.rs`):
//!
//! ```text
//! accrued      = half_up(B * bi') - half_up(B * bi)
//! fee          = half_up(accrued * rf / BPS)      rewards = accrued - fee
//! si'          = clamp(floor((half_up(S * si) + rewards) * RAY / S), si, MAX)
//! distributed  = half_up(S * si') - half_up(S * si)   shortfall = rewards - distributed
//! rev_shares   = floor((fee + shortfall) * RAY / si')  (capped at i128 headroom)
//! ```
//!
//! Documented bounds (formulas.md "Compounding and interest allocation",
//! "Accrual cadence", "Revenue payout"; INV-IDX-01/02/04/05; review note
//! "economic observations" 1 and 3):
//!
//! * per step, booked value (suppliers + treasury) never exceeds accrued
//!   interest by more than the half-up unit, and the stranded remainder is
//!   under `si'/RAY + 2` raw;
//! * a refinement of a partition never lowers the borrower's index or the
//!   treasury's value beyond per-step rounding (rate re-read at a higher
//!   utilization, truncated Taylor product dominates the single series);
//! * suppliers can lose to a refinement only the treasury's pro-rata
//!   participation in later rewards (revenue shares are supply shares);
//! * at the borrow-index ceiling nothing accrues; at the supply-index floor and
//!   ceiling the conservation bounds still hold;
//! * a revenue claim pays `min(cash, floor(value))` and burns a ceiling share
//!   count, so a dribble of partial claims never pays above the floor value.
//!
//! Every assertion is the documented bound; the printed lines carry the
//! measured drift (run with `--nocapture`).

use common::constants::{
    BPS, MAX_BORROW_INDEX_RAY, MAX_SUPPLY_INDEX_RAY, MILLISECONDS_PER_YEAR, RAY,
    SUPPLY_INDEX_FLOOR_RAW,
};
use common::math::fp::{Bps, Ray};
use common::math::fp_core::{mul_div_ceil, mul_div_floor};
use common::rates::{
    accrue_step, calculate_annual_borrow_rate, protocol_fee_shares, simulate_update_indexes,
    MAX_COMPOUND_DELTA_MS,
};
use common::types::{
    HubAssetKey, InterestRateModel, MarketParams, MarketParamsRaw, PoolKey, PoolStateRaw,
    PoolSyncData,
};
use proptest::prelude::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};
use test_harness::{hub_asset, LendingTest, MarketParamsPreset, DEFAULT_MARKET_PARAMS};

const YEAR_MS: u64 = MILLISECONDS_PER_YEAR;
const DAY_MS: u64 = 86_400_000;
/// 365 days: tiles 2, 24, 73, 365 and 8,760 equal chunks exactly.
const SPAN_MS: u64 = 365 * DAY_MS;
const HOUR_MS: u64 = 3_600_000;
const LEDGER_MS: u64 = 5_000;
const DECIMALS: u32 = 7;
/// One native unit of a 7-decimal token in RAY.
const UNIT_RAY: i128 = RAY / 10_000_000;

// ---------------------------------------------------------------------------
// Model: the production step over a partition, with every rounded intermediate
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct Book {
    borrowed: Ray,
    supplied: Ray,
    revenue: Ray,
    bi: Ray,
    si: Ray,
}

/// Per-run totals in raw RAY value units.
#[derive(Clone, Copy, Debug, Default)]
struct Ledger {
    steps: i128,
    accrued: i128,
    fee: i128,
    rewards: i128,
    distributed: i128,
    shortfall: i128,
    /// `half_up(rev_shares_i * si'_i)` summed over the steps.
    revenue_value: i128,
    /// `floor(rewards_i * revenue_i / supplied_i)`: the rewards that the
    /// treasury's existing shares take pro rata in step `i`.
    dilution: i128,
    max_si: i128,
    max_bi: i128,
    max_supplied: i128,
    elapsed_ms: u64,
}

impl Ledger {
    fn booked(&self) -> i128 {
        self.distributed + self.revenue_value
    }
    fn stranded(&self) -> i128 {
        self.accrued - self.booked()
    }
    /// Documented stranding bound: per step under `si'/RAY` (share floor) plus
    /// the half-up units of the two value differences.
    fn stranding_bound(&self) -> i128 {
        self.steps * (self.max_si / RAY + 2)
    }
    /// Rounding slack on the borrow index: `compound_interest` rounds seven
    /// terms half up (under 4 raw in the factor, scaled by `bi / RAY`),
    /// `update_borrow_index` rounds once more, and a re-read rate can sit one
    /// raw per-millisecond unit either side of the coarse rate (utilization
    /// half-up, the curve's one-unit kink steps, the per-ms division).
    fn borrow_index_slack(&self) -> i128 {
        let scale = self.max_bi / RAY + 1;
        self.steps * (4 * scale + 1) + 2 * (self.elapsed_ms as i128) * scale
    }
    /// Supplier rewards the supply-index floor leaves unrepresented per step,
    /// re-booked to the treasury: under `S / RAY` raw plus the half-up unit.
    fn shortfall_bound(&self) -> i128 {
        self.steps * (self.max_supplied / RAY + 1)
    }
}

/// Runs one caller-visible accrual of `delta_ms` (internally chunked by
/// `MAX_COMPOUND_DELTA_MS`, as `global_sync` does) and books every rounded
/// intermediate into `led`.
fn accrue(env: &Env, params: &MarketParams, book: &mut Book, led: &mut Ledger, delta_ms: u64) {
    let mut remaining = delta_ms;
    while remaining > 0 {
        let chunk = remaining.min(MAX_COMPOUND_DELTA_MS);
        let step = accrue_step(
            env,
            params,
            book.borrowed,
            book.supplied,
            book.bi,
            book.si,
            chunk,
        );

        let accrued = book
            .borrowed
            .mul(env, step.borrow_index)
            .checked_sub(env, book.borrowed.mul(env, book.bi));
        let fee = params.reserve_factor.apply_to_ray(env, accrued);
        let rewards = accrued.checked_sub(env, fee);
        let distributed = book
            .supplied
            .mul(env, step.supply_index)
            .checked_sub(env, book.supplied.mul(env, book.si));
        let shortfall = rewards.checked_sub(env, distributed);
        let revenue_value = step.revenue_shares.mul(env, step.supply_index);
        let dilution = if book.supplied == Ray::ZERO {
            0
        } else {
            mul_div_floor(env, rewards.raw(), book.revenue.raw(), book.supplied.raw())
        };

        led.steps += 1;
        led.accrued += accrued.raw();
        led.fee += fee.raw();
        led.rewards += rewards.raw();
        led.distributed += distributed.raw();
        led.shortfall += shortfall.raw();
        led.revenue_value += revenue_value.raw();
        led.dilution += dilution;
        led.max_si = led.max_si.max(step.supply_index.raw());
        led.max_bi = led.max_bi.max(step.borrow_index.raw());
        led.max_supplied = led.max_supplied.max(book.supplied.raw());
        led.elapsed_ms += chunk;

        book.bi = step.borrow_index;
        book.si = step.supply_index;
        book.supplied = book.supplied.checked_add(env, step.revenue_shares);
        book.revenue = book.revenue.checked_add(env, step.revenue_shares);
        remaining -= chunk;
    }
}

fn run(env: &Env, params: &MarketParams, start: Book, partition: &[u64]) -> (Book, Ledger) {
    let mut book = start;
    let mut led = Ledger::default();
    for &chunk in partition {
        accrue(env, params, &mut book, &mut led, chunk);
    }
    (book, led)
}

/// `n` chunks tiling `total_ms`; the last chunk takes the remainder.
fn equal_partition(total_ms: u64, n: u64) -> Vec<u64> {
    let base = total_ms / n;
    let mut v = vec![base; n as usize];
    *v.last_mut().unwrap() += total_ms - base * n;
    v
}

/// Pins the model's single-shot run to the production read path, which
/// `rv_accrual_consistency` pins to the mutator bit for bit.
fn pin_to_production(env: &Env, raw: &MarketParamsRaw, start: Book, total_ms: u64) {
    let sync = PoolSyncData {
        params: raw.clone(),
        state: PoolStateRaw {
            supplied: start.supplied.raw(),
            borrowed: start.borrowed.raw(),
            revenue: start.revenue.raw(),
            borrow_index: start.bi.raw(),
            supply_index: start.si.raw(),
            last_timestamp: 0,
            cash: 0,
        },
    };
    let production = simulate_update_indexes(env, total_ms, &sync);
    let (single, _) = run(env, &MarketParams::from(raw), start, &[total_ms]);
    assert_eq!(
        single.bi, production.borrow_index,
        "model pin: borrow index"
    );
    assert_eq!(
        single.si, production.supply_index,
        "model pin: supply index"
    );
}

/// Value accrued to the shares held at the start (the real suppliers: every
/// fixture starts with zero revenue shares).
fn supplier_gain(env: &Env, start: &Book, end: &Book) -> i128 {
    start
        .supplied
        .mul(env, end.si)
        .checked_sub(env, start.supplied.mul(env, start.si))
        .raw()
}

fn treasury_value(env: &Env, end: &Book) -> i128 {
    end.revenue.mul(env, end.si).raw()
}

fn borrower_cost(env: &Env, start: &Book, end: &Book) -> i128 {
    end.borrowed
        .mul(env, end.bi)
        .checked_sub(env, start.borrowed.mul(env, start.bi))
        .raw()
}

fn tokens(raw: i128) -> f64 {
    raw as f64 / RAY as f64
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn fresh_env() -> Env {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    env
}

fn raw_params(env: &Env, preset: &MarketParamsPreset) -> MarketParamsRaw {
    let raw = preset.to_market_params(&Address::generate(env), DECIMALS);
    raw.verify(env);
    raw
}

/// A curve that returns `rate + 1` raw for every positive utilization: base
/// and every slope at `rate`, maximum one raw unit above. `verify` admits it.
fn flat_preset(rate: i128) -> MarketParamsPreset {
    MarketParamsPreset {
        max_borrow_rate: rate + 1,
        base_borrow_rate: rate,
        slope1: rate,
        slope2: rate,
        slope3: rate,
        ..DEFAULT_MARKET_PARAMS
    }
}

/// A book whose supplied value is `supplied_tokens` whole tokens and whose
/// debt value is `util_bps` of it, with shares chosen at the given indexes.
fn book_at(env: &Env, supplied_tokens: i128, util_bps: i128, si: i128, bi: i128) -> Book {
    let supplied_value = Ray::from_asset(env, supplied_tokens * 10_000_000, DECIMALS);
    let debt_value = Bps::from(util_bps).apply_to_ray(env, supplied_value);
    Book {
        borrowed: debt_value.div_ceil(env, Ray::from(bi)),
        supplied: supplied_value.div_floor(env, Ray::from(si)),
        revenue: Ray::ZERO,
        bi: Ray::from(bi),
        si: Ray::from(si),
    }
}

/// Relative shortfall of the eighth-order Taylor series against `e^x`: the
/// remainder `sum_{k>=9} x^k / k!` over `e^x`, summed directly so that tails
/// far below f64 resolution of `1 - T8(x) / e^x` (5e-22 at x = 0.018) are
/// still exact to a few digits.
fn taylor_tail(x: f64) -> f64 {
    let mut term = 1.0;
    for k in 1..=9 {
        term *= x / k as f64;
    }
    let mut remainder = 0.0;
    let mut k = 9;
    while term > remainder * 1e-18 && k < 60 {
        remainder += term;
        k += 1;
        term *= x / k as f64;
    }
    remainder / x.exp()
}

/// Floating-point replay of the documented rule for the default curve.
fn annual_rate_f64(p: &MarketParamsPreset, util: f64) -> f64 {
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

fn f64_replay(p: &MarketParamsPreset, s: f64, b: f64, steps: u64, total_ms: u64) -> (f64, f64) {
    let dt_years = total_ms as f64 / steps as f64 / YEAR_MS as f64;
    let (mut supplied, mut bi, mut si) = (s, 1.0f64, 1.0f64);
    for _ in 0..steps {
        let util = (b * bi) / (supplied * si);
        let r = annual_rate_f64(p, util);
        let new_bi = bi * (r * dt_years).exp();
        let interest = b * (new_bi - bi);
        let fee = interest * p.reserve_factor as f64 / 10_000.0;
        let rewards = interest - fee;
        let new_si = (supplied * si + rewards) / supplied;
        supplied += fee / new_si;
        bi = new_bi;
        si = new_si;
    }
    (bi, si)
}

/// Shared assertions for a partitioned run against its one-shot baseline.
fn assert_partition_bounds(
    env: &Env,
    label: &str,
    start: &Book,
    base: &(Book, Ledger),
    part: &(Book, Ledger),
) {
    let (b_book, b_led) = base;
    let (p_book, p_led) = part;
    for (name, led) in [("base", b_led), ("part", p_led)] {
        assert!(
            led.booked() <= led.accrued + led.steps,
            "{label} {name}: booked {} exceeds accrued {} by more than {} steps",
            led.booked(),
            led.accrued,
            led.steps
        );
        assert!(
            led.stranded() <= led.stranding_bound(),
            "{label} {name}: stranded {} above bound {}",
            led.stranded(),
            led.stranding_bound()
        );
        assert!(
            led.revenue_value <= led.fee + led.shortfall + led.steps,
            "{label} {name}: revenue value {} above protocol reward {} + steps",
            led.revenue_value,
            led.fee + led.shortfall
        );
    }
    assert!(
        p_book.bi.raw() >= b_book.bi.raw() - p_led.borrow_index_slack(),
        "{label}: refinement lowered the borrow index {} -> {} beyond slack {}",
        b_book.bi.raw(),
        p_book.bi.raw(),
        p_led.borrow_index_slack()
    );
    // A borrow-index slack of `k` raw is worth `k * B / RAY` in debt value.
    let value_slack = mul_div_ceil(env, p_led.borrow_index_slack(), start.borrowed.raw(), RAY) + 1;
    let treasury_slack = p_led.stranding_bound() + p_led.steps + value_slack;
    assert!(
        treasury_value(env, p_book) >= treasury_value(env, b_book) - treasury_slack,
        "{label}: refinement lowered the treasury {} -> {} beyond slack {treasury_slack}",
        treasury_value(env, b_book),
        treasury_value(env, p_book)
    );
    let supplier_slack = p_led.dilution
        + p_led.shortfall_bound()
        + p_led.stranding_bound()
        + 2 * p_led.steps
        + value_slack;
    assert!(
        supplier_gain(env, start, p_book) >= supplier_gain(env, start, b_book) - supplier_slack,
        "{label}: suppliers lost {} to the refinement, above dilution {} + slack",
        supplier_gain(env, start, b_book) - supplier_gain(env, start, p_book),
        p_led.dilution
    );
}

// ---------------------------------------------------------------------------
// H1. Cadence decomposition over a year and a day: curve (utilization drift)
//     versus a flat rate (Taylor tail + revenue participation only).
// ---------------------------------------------------------------------------

#[test]
fn rv_r7_cadence_decomposes_into_drift_participation_and_taylor_tail() {
    let env = fresh_env();
    const SUPPLY_TOKENS: i128 = 1_000_000;
    for util_bps in [1_000i128, 6_000, 9_200] {
        let start = book_at(&env, SUPPLY_TOKENS, util_bps, RAY, RAY);
        let curve_raw = raw_params(&env, &DEFAULT_MARKET_PARAMS);
        let curve = MarketParams::from(&curve_raw);
        let r0 = calculate_annual_borrow_rate(&env, Ray::from(util_bps * RAY / BPS), &curve);
        let flat_p = flat_preset(r0.raw());
        let flat_raw = raw_params(&env, &flat_p);
        let flat = MarketParams::from(&flat_raw);

        for (family, raw, params) in [("curve", &curve_raw, &curve), ("flat", &flat_raw, &flat)] {
            pin_to_production(&env, raw, start, SPAN_MS);
            let base = run(&env, params, start, &[SPAN_MS]);
            for n in [2u64, 24, 73, 365, 8_760] {
                let part = run(&env, params, start, &equal_partition(SPAN_MS, n));
                let label = format!("{family} util {}% n={n}", util_bps / 100);
                assert_partition_bounds(&env, &label, &start, &base, &part);

                let d_cost =
                    borrower_cost(&env, &start, &part.0) - borrower_cost(&env, &start, &base.0);
                let d_sup =
                    supplier_gain(&env, &start, &part.0) - supplier_gain(&env, &start, &base.0);
                let d_tre = treasury_value(&env, &part.0) - treasury_value(&env, &base.0);
                let rel_bi = (part.0.bi.raw() - base.0.bi.raw()) as f64 / base.0.bi.raw() as f64;
                println!(
                    "H1 {label}: bi 1={} n={} rel {rel_bi:.3e} | borrower +{:.9} supplier {:+.9} treasury {:+.9} tokens | dilution {:.9} stranded base {} part {} raw",
                    base.0.bi.raw(),
                    part.0.bi.raw(),
                    tokens(d_cost),
                    tokens(d_sup),
                    tokens(d_tre),
                    tokens(part.1.dilution),
                    base.1.stranded(),
                    part.1.stranded()
                );

                if family == "flat" {
                    // Only the Taylor tail separates the partitions: the single
                    // series undershoots e^x by tail(x); n sub-series undershoot
                    // by about tail(x/n) each.
                    let x = (r0.raw() + 1) as f64 / RAY as f64 * SPAN_MS as f64 / YEAR_MS as f64;
                    let tail = taylor_tail(x);
                    assert!(
                        rel_bi <= tail * 1.001 + 1e-20,
                        "{label}: flat-rate spread {rel_bi:e} above the Taylor tail {tail:e}"
                    );
                    assert!(
                        rel_bi >= -1e-20,
                        "{label}: flat-rate refinement lowered the index ({rel_bi:e})"
                    );
                    // Suppliers lose exactly the treasury's participation (up to rounding).
                    assert!(
                        d_sup <= 2 * part.1.steps,
                        "{label}: flat rate must not raise the supplier gain ({d_sup})"
                    );
                } else {
                    let (fb_bi, fb_si) = f64_replay(
                        &DEFAULT_MARKET_PARAMS,
                        SUPPLY_TOKENS as f64,
                        (SUPPLY_TOKENS * util_bps / BPS) as f64,
                        n,
                        SPAN_MS,
                    );
                    let rel_fb = (part.0.bi.raw() as f64 / RAY as f64 - fb_bi).abs() / fb_bi;
                    let rel_fs = (part.0.si.raw() as f64 / RAY as f64 - fb_si).abs() / fb_si;
                    // The per-chunk Taylor tail (x/n up to about 0.6) bounds the gap.
                    assert!(
                        rel_fb <= 1e-7 && rel_fs <= 1e-7,
                        "{label}: integer path drifts from the documented rule by {rel_fb:e} / {rel_fs:e}"
                    );
                }
            }
        }
    }

    // One day at ledger cadence (17,280 steps) on the curve.
    for util_bps in [6_000i128, 9_200] {
        let start = book_at(&env, SUPPLY_TOKENS, util_bps, RAY, RAY);
        let raw = raw_params(&env, &DEFAULT_MARKET_PARAMS);
        let params = MarketParams::from(&raw);
        pin_to_production(&env, &raw, start, DAY_MS);
        let base = run(&env, &params, start, &[DAY_MS]);
        for n in [24u64, DAY_MS / LEDGER_MS] {
            let part = run(&env, &params, start, &equal_partition(DAY_MS, n));
            let label = format!("curve day util {}% n={n}", util_bps / 100);
            assert_partition_bounds(&env, &label, &start, &base, &part);
            println!(
                "H1 {label}: borrower +{:.12} supplier {:+.12} treasury {:+.12} tokens | stranded base {} part {} raw (steps {})",
                tokens(borrower_cost(&env, &start, &part.0) - borrower_cost(&env, &start, &base.0)),
                tokens(supplier_gain(&env, &start, &part.0) - supplier_gain(&env, &start, &base.0)),
                tokens(treasury_value(&env, &part.0) - treasury_value(&env, &base.0)),
                base.1.stranded(),
                part.1.stranded(),
                part.1.steps
            );
        }
    }
}

// ---------------------------------------------------------------------------
// H2. Random partitions and refinements within one year, across indexes.
// ---------------------------------------------------------------------------

/// Cuts `total_ms` at the sorted `fractions` (per mille); refines by adding cuts.
fn partition_from_cuts(total_ms: u64, cuts_pm: &[u16]) -> Vec<u64> {
    let mut points: Vec<u64> = cuts_pm
        .iter()
        .map(|&c| (total_ms as u128 * c as u128 / 1_000) as u64)
        .collect();
    points.push(0);
    points.push(total_ms);
    points.sort_unstable();
    points.dedup();
    points.windows(2).map(|w| w[1] - w[0]).collect()
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 48,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn rv_r7_random_refinement_keeps_documented_direction(
        total_ms in HOUR_MS..=YEAR_MS,
        coarse in prop::collection::vec(1u16..1_000, 1..6),
        extra in prop::collection::vec(1u16..1_000, 1..8),
        util_bps in 500i128..=9_500,
        supply_tokens in prop_oneof![Just(1_000i128), Just(250_000), Just(10_000_000)],
        si in prop_oneof![
            Just(SUPPLY_INDEX_FLOOR_RAW),
            Just(RAY - 1),
            Just(RAY),
            Just(RAY * 3 / 2 + 7),
            Just(RAY * 1_000),
            Just(MAX_SUPPLY_INDEX_RAY / 10),
        ],
        bi in prop_oneof![
            Just(RAY),
            Just(RAY + 1),
            Just(RAY * 7 / 5),
            Just(RAY * 1_000_000),
            Just(MAX_BORROW_INDEX_RAY / 1_000),
        ],
    ) {
        let env = fresh_env();
        let raw = raw_params(&env, &DEFAULT_MARKET_PARAMS);
        let params = MarketParams::from(&raw);
        let start = book_at(&env, supply_tokens, util_bps, si, bi);
        pin_to_production(&env, &raw, start, total_ms);

        let single = run(&env, &params, start, &[total_ms]);
        let coarse_p = partition_from_cuts(total_ms, &coarse);
        let mut fine_cuts = coarse.clone();
        fine_cuts.extend_from_slice(&extra);
        let fine_p = partition_from_cuts(total_ms, &fine_cuts);
        let coarse_r = run(&env, &params, start, &coarse_p);
        let fine_r = run(&env, &params, start, &fine_p);

        assert_partition_bounds(&env, "coarse vs single", &start, &single, &coarse_r);
        assert_partition_bounds(&env, "fine vs coarse", &start, &coarse_r, &fine_r);
        assert_partition_bounds(&env, "fine vs single", &start, &single, &fine_r);
        prop_assert!(fine_r.0.si.raw() >= SUPPLY_INDEX_FLOOR_RAW && fine_r.0.si.raw() <= MAX_SUPPLY_INDEX_RAY);
        prop_assert!(fine_r.0.bi.raw() >= bi && fine_r.0.bi.raw() <= MAX_BORROW_INDEX_RAY);
    }
}

// ---------------------------------------------------------------------------
// H3. Borrow-index ceiling, supply-index floor and ceiling.
// ---------------------------------------------------------------------------

#[test]
fn rv_r7_index_ceiling_and_floor_steps_stay_conservative() {
    let env = fresh_env();
    let raw = raw_params(&env, &DEFAULT_MARKET_PARAMS);
    let params = MarketParams::from(&raw);

    // (a) Ceiling: 100 tokens of debt shares at bi = 0.5e36, 200% rate (utilization
    // clamps to one). One year would multiply the index by e^2; the cap keeps it at 1e36,
    // so the borrower is charged exactly B * 0.5e9 and nothing afterwards.
    let b = Ray::from(100 * 10_000_000 * UNIT_RAY);
    let mut cap_book = Book {
        borrowed: b,
        supplied: b,
        revenue: Ray::ZERO,
        bi: Ray::from(MAX_BORROW_INDEX_RAY / 2),
        si: Ray::ONE,
    };
    let mut led = Ledger::default();
    accrue(&env, &params, &mut cap_book, &mut led, YEAR_MS);
    assert_eq!(
        cap_book.bi.raw(),
        MAX_BORROW_INDEX_RAY,
        "index sits on the ceiling"
    );
    let charged = b.mul(&env, Ray::from(MAX_BORROW_INDEX_RAY / 2)).raw();
    assert_eq!(
        led.accrued, charged,
        "interest is exactly the index room left"
    );
    assert!(led.booked() <= led.accrued + 1 && led.stranded() <= led.stranding_bound());
    let frozen = cap_book;
    let mut led2 = Ledger::default();
    accrue(&env, &params, &mut cap_book, &mut led2, YEAR_MS);
    assert_eq!(led2.accrued, 0, "no interest at the ceiling");
    assert_eq!(
        cap_book.revenue, frozen.revenue,
        "no revenue at the ceiling"
    );
    assert_eq!(cap_book.si, frozen.si, "no supplier reward at the ceiling");
    println!(
        "H3a ceiling: charged {} raw ({:.3} tokens), booked {} stranded {}; second year accrued 0",
        led.accrued,
        tokens(led.accrued),
        led.booked(),
        led.stranded()
    );

    // (b) Supply-index floor 1e24: 1M tokens supplied, 60% utilization, a year in one
    // step and in 365 daily steps. Revenue shares are 1000x the value; the stranding
    // bound is two raw units per step because si'/RAY is zero.
    let start = book_at(&env, 1_000_000, 6_000, SUPPLY_INDEX_FLOOR_RAW, RAY);
    pin_to_production(&env, &raw, start, YEAR_MS);
    let single = run(&env, &params, start, &[YEAR_MS]);
    let daily = run(&env, &params, start, &equal_partition(YEAR_MS, 365));
    assert_partition_bounds(&env, "floor", &start, &single, &daily);
    assert!(
        single.0.si.raw() >= SUPPLY_INDEX_FLOOR_RAW && daily.0.si.raw() >= SUPPLY_INDEX_FLOOR_RAW
    );
    assert_eq!(daily.1.stranding_bound(), 2 * 365);
    println!(
        "H3b floor: single si {} daily si {} stranded {} / {} raw, revenue shares {} / {}",
        single.0.si.raw(),
        daily.0.si.raw(),
        single.1.stranded(),
        daily.1.stranded(),
        single.0.revenue.raw(),
        daily.0.revenue.raw()
    );

    // (c) Supply-index ceiling 1e36: every reward is shortfall and goes to the treasury
    // at floor(reward / 1e9) shares; under 1e9 raw (1e-18 tokens) strands per step.
    let start = book_at(&env, 1_000_000, 6_000, MAX_SUPPLY_INDEX_RAY, RAY);
    pin_to_production(&env, &raw, start, 100 * DAY_MS);
    let (end, led) = run(&env, &params, start, &equal_partition(100 * DAY_MS, 100));
    assert_eq!(
        end.si.raw(),
        MAX_SUPPLY_INDEX_RAY,
        "index stays on the ceiling"
    );
    assert_eq!(
        led.distributed, 0,
        "suppliers receive nothing at the ceiling"
    );
    assert_eq!(led.shortfall, led.rewards, "every reward is shortfall");
    assert!(led.booked() <= led.accrued + led.steps && led.stranded() <= led.stranding_bound());
    println!(
        "H3c supply ceiling: accrued {} treasury {} stranded {} raw ({:.3e} tokens) over {} steps",
        led.accrued,
        led.revenue_value,
        led.stranded(),
        tokens(led.stranded()),
        led.steps
    );
}

// ---------------------------------------------------------------------------
// H4. Flash-loan fee booked as revenue in a loop: cash is never below the claim.
// ---------------------------------------------------------------------------

#[test]
fn rv_r7_flash_fee_loop_leaves_cash_at_or_above_the_revenue_claim() {
    let env = fresh_env();
    for si in [RAY + 7, SUPPLY_INDEX_FLOOR_RAW, MAX_SUPPLY_INDEX_RAY - 1] {
        let index = Ray::from(si);
        let mut supplied =
            Ray::from_asset(&env, 1_000 * 10_000_000, DECIMALS).div_floor(&env, index);
        let mut revenue = Ray::ZERO;
        let mut cash = 0i128;
        for amount in [1i128, 9_999, 10_000, 123_456_789] {
            for _ in 0..2_500 {
                // ops/flash.rs:108 then interest.rs:64: half-up fee, minimum one unit,
                // floor-converted into shares at the current supply index.
                let fee = Bps::from(1).flash_loan_fee_on(&env, amount);
                let shares = protocol_fee_shares(
                    &env,
                    Ray::from_asset(&env, fee, DECIMALS),
                    index,
                    supplied,
                );
                supplied = supplied.checked_add(&env, shares);
                revenue = revenue.checked_add(&env, shares);
                cash += fee;
            }
        }
        let claim = revenue
            .mul_floor(&env, index)
            .to_asset_floor(&env, DECIMALS);
        assert!(claim <= cash, "si {si}: claim {claim} above cash {cash}");
        assert!(
            claim >= cash - 1,
            "si {si}: claim {claim} short of cash {cash} by over a unit"
        );
        println!("H4 flash fee loop si {si}: 10,000 loans, cash {cash} units, claim {claim} units");
    }
}

// ---------------------------------------------------------------------------
// H5. Revenue claim dribble through the deployed pool: 1 unit of cash at a time.
// ---------------------------------------------------------------------------

#[test]
fn rv_r7_claim_revenue_dribble_through_pool_never_pays_above_floor_value() {
    let mut preset = test_harness::usdc_preset();
    preset.initial_liquidity = 0.0;
    let t = LendingTest::new().with_market(preset).build();
    let env = t.env.clone();
    let market = t.resolve_market("USDC");
    let key: HubAssetKey = hub_asset(market.asset.clone());
    let pool_addr = market.pool.clone();
    let pl = t.pool_client("USDC");
    let mut model: InterestRateModel = pl.get_sync_data(&key).params.rate_model_view();
    model.max_utilization = RAY;
    pl.update_params(&key, &model);

    // 300.7 units of revenue at an odd index; cash arrives one unit at a time.
    let si: i128 = 1_234_567_891_234_567_891_234_567_891;
    let revenue0 = Ray::from(3_007 * UNIT_RAY / 10).div_ceil(&env, Ray::from(si));
    let initial_floor = Ray::from(revenue0.raw())
        .mul_floor(&env, Ray::from(si))
        .to_asset_floor(&env, DECIMALS);
    assert_eq!(initial_floor, 300);
    let now_ms = env.ledger().timestamp() * 1_000;
    let inject = |revenue: i128, cash: i128| {
        env.as_contract(&pool_addr, || {
            env.storage().persistent().set(
                &PoolKey::State(key.clone()),
                &PoolStateRaw {
                    supplied: revenue,
                    borrowed: 0,
                    revenue,
                    borrow_index: RAY,
                    supply_index: si,
                    last_timestamp: now_ms,
                    cash,
                },
            );
        });
    };

    let mut revenue = revenue0.raw();
    let mut paid = 0i128;
    let mut claims = 0u32;
    while revenue > 0 {
        let treasury = Ray::from(revenue)
            .mul_floor(&env, Ray::from(si))
            .to_asset_floor(&env, DECIMALS);
        if treasury == 0 {
            break;
        }
        let cash = if claims < 200 { 1 } else { 1_000 };
        inject(revenue, cash);
        market.token_admin.mint(&pool_addr, &cash);
        let amount = cash.min(treasury);
        // shares.rs:60-65: full burn at the floor value, else ceil(revenue * amount / treasury).
        let expect_burn = if amount >= treasury {
            revenue
        } else {
            mul_div_ceil(&env, revenue, amount, treasury)
        };
        let got = pl.claim_revenue(&key).actual_amount;
        let after = pl.get_sync_data(&key).state;
        assert_eq!(
            got, amount,
            "claim {claims}: payout is min(cash, floor value)"
        );
        assert_eq!(
            revenue - after.revenue,
            expect_burn,
            "claim {claims}: ceil burn"
        );
        assert_eq!(
            after.supplied, after.revenue,
            "claim {claims}: supply follows revenue"
        );
        assert_eq!(after.cash, cash - amount, "claim {claims}: cash debited");
        revenue = after.revenue;
        paid += got;
        claims += 1;
    }
    assert!(
        paid <= initial_floor,
        "dribble paid {paid} above the floor value {initial_floor}"
    );
    assert!(
        paid >= initial_floor - 1,
        "dribble paid {paid}, floor value {initial_floor}"
    );
    println!(
        "H5 claim dribble: {claims} claims paid {paid} of floor {initial_floor} units; residual shares {revenue} (value floor 0)"
    );
}
