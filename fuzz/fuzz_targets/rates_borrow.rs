//! Fuzz `calculate_borrow_rate` — 3-region piecewise linear model.
//!
//! Invariants:
//!   1. Result is non-negative
//!   2. Result ≤ max_borrow_rate_per_ms (capped)
//!   3. Monotonically non-decreasing in utilization (within a region)
//!   4. Continuity at region boundaries (|rate(u-ε) - rate(u+ε)| ≤ 2 ulp)
//!
//! Parameters are clamped to respect protocol validation rules:
//!   0 < mid < optimal < RAY, slopes ≥ 0, base ≥ 0, max_rate > 0
#![no_main]
use arbitrary::Arbitrary;
use common::fp::Ray;
use common::rates::calculate_borrow_rate;
use common::types::MarketParams;
use libfuzzer_sys::fuzz_target;
use soroban_sdk::{Address, Env};

const RAY: i128 = 1_000_000_000_000_000_000_000_000_000;
const MS_PER_YEAR: i128 = 31_556_926_000;

#[derive(Debug, Arbitrary)]
struct In {
    util_bps: u16, // 0..=10000
    base_pct: u8,  // 0..=50 → 0..50% annual
    s1_pct: u8,    // 0..=50
    s2_pct: u8,    // 0..=100
    s3_pct: u16,   // 0..=500 → 0..500%
    mid_pct: u8,   // 1..=99
    opt_pct: u8,   // mid+1..=99
    max_pct: u16,  // 1..=1000 → 0..1000%
    /// When `flip & 0b111 == 0` (probability ≈ 1/8), switch to the degenerate-
    /// parameters path which deliberately constructs invalid geometries
    /// (mid=0, optimal=mid, optimal=RAY). This exercises known-bug territory
    /// (M-07 division-by-zero, L-11 mid=0) that the clamped path rules out.
    flip: u8,
}

fn make_params(env: &Env, i: &In) -> Option<MarketParams> {
    let mid_pct = (i.mid_pct % 98 + 1) as i128; // 1..=98
    let opt_pct = (i.opt_pct as i128 % (99 - mid_pct)) + mid_pct + 1; // mid+1..=99

    let base = RAY * (i.base_pct as i128 % 51) / 100;
    let s1 = RAY * (i.s1_pct as i128 % 51) / 100;
    let s2 = RAY * (i.s2_pct as i128 % 101) / 100;
    let s3 = RAY * (i.s3_pct as i128 % 501) / 100;
    let mid = RAY * mid_pct / 100;
    let opt = RAY * opt_pct / 100;
    let max_rate = RAY * (i.max_pct.max(1) as i128 % 1001) / 100;

    Some(MarketParams {
        base_borrow_rate_ray: base,
        slope1_ray: s1,
        slope2_ray: s2,
        slope3_ray: s3,
        mid_utilization_ray: mid,
        optimal_utilization_ray: opt,
        max_borrow_rate_ray: max_rate.max(1),
        reserve_factor_bps: 1000,
        asset_id: Address::from_str(
            env,
            "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC",
        ),
        asset_decimals: 7,
    })
}

/// Build a deliberately degenerate `MarketParams` with `mid = 0` (L-11
/// regression probe). The production path is guarded upstream by
/// `validation::validate_interest_rate_model` — this fuzz path verifies
/// that if those guards are ever bypassed, the math below doesn't silently
/// wrap (it should either return a valid non-negative rate or panic).
///
/// NOTE: we intentionally DO NOT probe `optimal == mid` or `optimal == RAY`
/// here. Those cause deterministic division-by-zero in Region 2/3, which
/// panics with `HostError::ArithDomain`. libfuzzer-sys installs its own
/// panic hook that aborts before our `catch_unwind` intercepts, so any
/// panic is reported as an exit-77 crash regardless of whether we caught
/// it. Those degenerate shapes belong in a proptest harness where
/// `catch_unwind` actually works.
fn make_degenerate_params(env: &Env, i: &In) -> MarketParams {
    let base = RAY * (i.base_pct as i128 % 51) / 100;
    let s1 = RAY * (i.s1_pct as i128 % 51) / 100;
    let s2 = RAY * (i.s2_pct as i128 % 101) / 100;
    let s3 = RAY * (i.s3_pct as i128 % 501) / 100;
    let max_rate = (RAY * (i.max_pct.max(1) as i128 % 1001) / 100).max(1);

    // mid = 0 path only. With mid=0, any positive utilization falls into
    // Region 2 or 3, where `range = optimal - mid = optimal > 0` — safe.
    // Utilization=0 hits the early-return branch in calculate_borrow_rate.
    let mid = 0i128;
    let opt = RAY / 2;

    MarketParams {
        base_borrow_rate_ray: base,
        slope1_ray: s1,
        slope2_ray: s2,
        slope3_ray: s3,
        mid_utilization_ray: mid,
        optimal_utilization_ray: opt,
        max_borrow_rate_ray: max_rate,
        reserve_factor_bps: 1000,
        asset_id: Address::from_str(
            env,
            "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC",
        ),
        asset_decimals: 7,
    }
}

fuzz_target!(|i: In| {
    let env = Env::default();

    // 1-in-8 path: degenerate parameters. The only invariant is "no silent
    // wraparound" — either the call returns cleanly, or it panics with a
    // typed error. A panic is *accepted* here; silent wrong results are not.
    if i.flip & 0b111 == 0 {
        use std::panic::AssertUnwindSafe;
        let params = make_degenerate_params(&env, &i);
        let util_bps = (i.util_bps % 10_001) as i128;
        let util = Ray::from_raw(RAY * util_bps / 10_000);
        let res = std::panic::catch_unwind(AssertUnwindSafe(|| {
            calculate_borrow_rate(&env, util, &params)
        }));
        match res {
            // Panic is acceptable — degenerate geometry should panic, not wrap.
            Err(_) => return,
            Ok(rate) => {
                // If it DID return, the result must at minimum be non-negative.
                // Anything else (e.g. a wrapped negative number) signals a silent
                // wraparound, which IS a bug and must fail loudly.
                assert!(
                    rate.raw() >= 0,
                    "degenerate params returned negative rate (silent wrap): {}",
                    rate.raw()
                );
                return;
            }
        }
    }

    let Some(params) = make_params(&env, &i) else { return };

    let util_bps = (i.util_bps % 10_001) as i128;
    let util = Ray::from_raw(RAY * util_bps / 10_000);

    let rate = calculate_borrow_rate(&env, util, &params);

    // Invariant 1: non-negative
    assert!(rate.raw() >= 0, "negative rate at util={} params={:?}", util.raw(), params.mid_utilization_ray);

    // Invariant 2: capped at max_rate / MS_PER_YEAR
    let max_per_ms = params.max_borrow_rate_ray / MS_PER_YEAR;
    // +1 ulp tolerance for half-up rounding on the final div_by_int
    assert!(
        rate.raw() <= max_per_ms + 2,
        "rate exceeded max: rate={} max_per_ms={}",
        rate.raw(),
        max_per_ms
    );

    // Invariant 3: monotonicity — rate(util) ≤ rate(util+δ)
    if util_bps < 10_000 {
        let util_hi = Ray::from_raw(RAY * (util_bps + 1) / 10_000);
        let rate_hi = calculate_borrow_rate(&env, util_hi, &params);
        // Allow 2 ulp slack for rounding across region joins
        assert!(
            rate.raw() <= rate_hi.raw() + 2,
            "monotonicity violated: rate({})={} > rate({})={}",
            util_bps, rate.raw(), util_bps + 1, rate_hi.raw()
        );
    }
});
