//! Unified fuzz target for the rate → compound → accrual pipeline.
//!
//! Checks the protocol-level interest-split identity across the
//! rate, compound-interest, and accrual pipeline:
//!
//!   accrued_interest = supplier_rewards + protocol_fee        (exact)
//!   protocol_fee ≈ reserve_factor_bps/BPS × accrued_interest  (±1 ulp)
//!
//! Pipeline per iteration:
//!   1. Sample `MarketParams` (clamped to validator-valid geometry).
//!   2. `rate = calculate_borrow_rate(util, params)`.
//!   3. `factor = compound_interest(rate, delta_ms)`.
//!   4. `(rewards, fee) = calculate_supplier_rewards(params, borrowed, factor, RAY)`.
//!   5. Assert all rate / compound / accrual invariants.
//!
//! ### Why no `catch_unwind`
//!
//! libfuzzer-sys's panic hook calls `std::process::abort()` *before* the
//! unwind reaches any `catch_unwind`, so panics cannot be recovered inside
//! the target. Inputs are therefore clamped to the production-valid domain:
//!   - `x = rate · delta_ms ≤ 2 RAY` — the Taylor expansion's designed bound
//!     (error < 0.01% at x = 2); beyond that the real protocol caps rates.
//!   - `borrowed_raw ≤ 10^17` — keeps `borrowed · factor` in-range even when
//!     `factor` approaches `e^2`.
//!
//! Anything outside those bounds is skipped, not asserted.
#![no_main]
use arbitrary::Arbitrary;
use common::constants::{BPS, MILLISECONDS_PER_YEAR, RAY};
use common::fp::Ray;
use common::rates::{
    calculate_borrow_rate, calculate_deposit_rate, calculate_supplier_rewards, compound_interest,
    simulate_update_indexes,
};
use common::types::MarketParams;
use libfuzzer_sys::fuzz_target;
use soroban_sdk::{Address, Env};

const MS_PER_YEAR: u64 = MILLISECONDS_PER_YEAR;

/// 29-byte layout. The first fields cover rate-model geometry; the tail
/// carries accrual inputs: `reserve_pct`, `delta_ms`, `borrowed_units`.
#[derive(Debug, Arbitrary)]
struct In {
    // --- rate params ---
    util_bps: u16,
    base_pct: u8,
    s1_pct: u8,
    s2_pct: u8,
    s3_pct: u16,
    mid_pct: u8,
    opt_pct: u8,
    max_pct: u16,
    flip: u8, // reserved; preserved for seed-corpus bit-compat
    // --- accrual ---
    reserve_pct: u8,
    delta_ms: u64,
    borrowed_units: u64,
    // --- pipeline wrapper (simulate_update_indexes) ---
    // Tail fields allow short corpus entries to deserialize to zero and hit
    // skip paths deterministically.
    supplied_units: u64,
}

fn make_params(env: &Env, i: &In) -> MarketParams {
    let mid_pct = (i.mid_pct % 98 + 1) as i128; // 1..=98
    let opt_pct = (i.opt_pct as i128 % (99 - mid_pct)) + mid_pct + 1; // mid+1..=99

    let base = RAY * (i.base_pct as i128 % 51) / 100;
    let s1 = RAY * (i.s1_pct as i128 % 51) / 100;
    let s2 = RAY * (i.s2_pct as i128 % 101) / 100;
    let s3 = RAY * (i.s3_pct as i128 % 501) / 100;
    let mid = RAY * mid_pct / 100;
    let opt = RAY * opt_pct / 100;
    let max_rate = RAY * (i.max_pct.max(1) as i128 % 1001) / 100;
    let reserve_factor_bps = ((i.reserve_pct as i128 % 51) * 100).clamp(0, BPS - 1);

    MarketParams {
        base_borrow_rate_ray: base,
        slope1_ray: s1,
        slope2_ray: s2,
        slope3_ray: s3,
        mid_utilization_ray: mid,
        optimal_utilization_ray: opt,
        max_borrow_rate_ray: max_rate.max(1),
        reserve_factor_bps,
        asset_id: Address::from_str(
            env,
            "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC",
        ),
        asset_decimals: 7,
    }
}

fn assert_rate_invariants(env: &Env, util_bps: i128, params: &MarketParams, rate: Ray) {
    assert!(
        rate.raw() >= 0,
        "negative rate at util_bps={} mid={}",
        util_bps,
        params.mid_utilization_ray
    );

    let max_per_ms = params.max_borrow_rate_ray / MS_PER_YEAR as i128;
    assert!(
        rate.raw() <= max_per_ms + 2,
        "rate exceeded max: rate={} max_per_ms={}",
        rate.raw(),
        max_per_ms
    );

    // Monotonically non-decreasing in utilization (2 ulp slack across joins).
    if util_bps < 10_000 {
        let util_hi = Ray::from_raw(RAY * (util_bps + 1) / 10_000);
        let rate_hi = calculate_borrow_rate(env, util_hi, params);
        assert!(
            rate.raw() <= rate_hi.raw() + 2,
            "monotonicity violated: rate({})={} > rate({})={}",
            util_bps,
            rate.raw(),
            util_bps + 1,
            rate_hi.raw()
        );
    }
}

fn assert_compound_invariants(env: &Env, rate: Ray, delta_ms: u64, factor: Ray) {
    if delta_ms == 0 {
        assert_eq!(factor, Ray::ONE);
        return;
    }

    assert!(
        factor.raw() >= RAY - 1,
        "compound below 1.0: factor={} rate={} dt={}",
        factor.raw(),
        rate.raw(),
        delta_ms
    );

    // Monotonic in delta: factor(t) ≥ factor(t-1).
    let prev = compound_interest(env, rate, delta_ms - 1);
    assert!(
        factor.raw() >= prev.raw(),
        "non-monotonic in delta: f(t-1)={} f(t)={}",
        prev.raw(),
        factor.raw()
    );

    // Taylor floor: factor ≥ 1 + r·t (omitted terms non-negative).
    if let Some(rt) = rate.raw().checked_mul(delta_ms as i128) {
        if let Some(linear_floor) = RAY.checked_add(rt) {
            assert!(
                factor.raw() >= linear_floor - 2,
                "Taylor below linear floor: factor={} 1+r*t={}",
                factor.raw(),
                linear_floor
            );
        }
    }
}

fn assert_interest_split(
    env: &Env,
    params: &MarketParams,
    borrowed: Ray,
    new_index: Ray,
    old_index: Ray,
) {
    let (rewards, fee) = calculate_supplier_rewards(env, params, borrowed, new_index, old_index);

    assert!(
        rewards.raw() >= 0 && fee.raw() >= 0,
        "negative split: rewards={} fee={}",
        rewards.raw(),
        fee.raw()
    );

    let old_debt = borrowed.mul(env, old_index);
    let new_debt = borrowed.mul(env, new_index);
    let accrued = new_debt - old_debt;

    // Exact conservation: rewards + fee == accrued.
    assert_eq!(
        rewards.raw() + fee.raw(),
        accrued.raw(),
        "§5 conservation: rewards={} + fee={} != accrued={}",
        rewards.raw(),
        fee.raw(),
        accrued.raw()
    );

    // Zero reserve factor ⇒ no protocol fee.
    if params.reserve_factor_bps == 0 {
        assert_eq!(fee.raw(), 0, "fee non-zero with reserve_factor_bps=0");
    }

    // Half-up bound on Bps::apply_to: |fee*BPS - rf*accrued| ≤ BPS/2 + 1.
    if let (Some(fee_scaled), Some(rf_scaled)) = (
        fee.raw().checked_mul(BPS),
        params.reserve_factor_bps.checked_mul(accrued.raw()),
    ) {
        let err = (fee_scaled - rf_scaled).abs();
        assert!(
            err <= BPS / 2 + 1,
            "§5 fee rounding: fee*BPS={} rf*accrued={} err={} rf={}",
            fee_scaled,
            rf_scaled,
            err,
            params.reserve_factor_bps
        );
    }
}

fuzz_target!(|i: In| {
    let _ = i.flip; // Reserved corpus byte.
    let env = Env::default();

    let params = make_params(&env, &i);
    let util_bps = (i.util_bps % 10_001) as i128;
    let util = Ray::from_raw(RAY * util_bps / 10_000);

    let rate = calculate_borrow_rate(&env, util, &params);
    assert_rate_invariants(&env, util_bps, &params, rate);

    // Clamp `x = rate * delta_ms` into the Taylor expansion's accurate range
    // (<= 2 RAY). Outside that, production rate caps would intervene; skip
    // out-of-domain inputs to avoid `MathOverflow` panics in the harness.
    let delta_ms = i.delta_ms % (10 * MS_PER_YEAR);
    match rate.raw().checked_mul(delta_ms as i128) {
        Some(rt) if rt.abs() <= 2 * RAY => {}
        _ => return,
    }

    let factor = compound_interest(&env, rate, delta_ms);
    assert_compound_invariants(&env, rate, delta_ms, factor);

    // §5 split: old_index pinned at RAY so `new_index = factor`.
    // Cap borrowed_raw so `borrowed * factor ≈ 10^17 * 7.4 RAY` stays within
    // i128 when converted back to RAY units.
    const BORROW_CAP_RAW: i128 = 100_000_000_000_000_000;
    let borrowed_raw = (i.borrowed_units as i128).min(BORROW_CAP_RAW);
    if borrowed_raw <= 0 {
        return;
    }
    assert_interest_split(&env, &params, Ray::from_raw(borrowed_raw), factor, Ray::ONE);

    // ---- calculate_deposit_rate (not reachable via simulate_update_indexes) ----
    // `util` and `rate` are already clamped into the validated domain above.
    let deposit_rate = calculate_deposit_rate(&env, util, rate, params.reserve_factor_bps);
    assert!(
        deposit_rate.raw() >= 0,
        "negative deposit rate: {}",
        deposit_rate.raw()
    );
    // Supplier share is (1 - reserve_factor) × utilization × borrow_rate ≤ borrow_rate.
    // 1-ulp slack for Bps::apply_to's half-up rounding.
    assert!(
        deposit_rate.raw() <= rate.raw() + 1,
        "deposit rate > borrow rate: dep={} bor={}",
        deposit_rate.raw(),
        rate.raw()
    );
    // reserve_factor == BPS-1 clamp edge: denominator (BPS - rf) = 1, so the
    // deposit rate can approach rate; still must not exceed it.
    if params.reserve_factor_bps == 0 && util.raw() > 0 {
        // Zero reserve factor ⇒ suppliers get the full util × rate (± 1 ulp).
        let expected = rate.mul(&env, util);
        let diff = (deposit_rate.raw() - expected.raw()).abs();
        assert!(
            diff <= 1,
            "deposit rate mismatch with rf=0: dep={} expected={} diff={}",
            deposit_rate.raw(),
            expected.raw(),
            diff
        );
    }

    // ---- simulate_update_indexes (pipeline wrapper) ----
    // Exercises `utilization`, `scaled_to_original`, `update_borrow_index`,
    // `update_supply_index` — functions otherwise unreached.
    //
    // Domain: `simulate_update_indexes` recomputes utilization internally
    // from `borrowed/supplied`. Derive `supplied` so the internal util
    // matches the already-validated `util_bps`, keeping us inside the
    // `rate * delta_ms <= 2 RAY` bound enforced above. Otherwise an
    // adversarial supplied/borrowed ratio can produce a region-3 rate that
    // overflows compound_interest.
    // util_bps == 0 can't be faithfully reproduced via borrowed/supplied
    // (would need supplied → ∞). Skip rather than assert-with-caveats.
    if util_bps == 0 {
        return;
    }

    // supplied = borrowed * 10_000 / util_bps — reproduces the same util so
    // simulate_update_indexes' internal rate falls inside the clamped domain.
    const SUPPLY_CAP_RAW: i128 = 200_000_000_000_000_000;
    let supplied_raw = match borrowed_raw.checked_mul(10_000) {
        Some(scaled) => (scaled / util_bps).min(SUPPLY_CAP_RAW),
        None => return,
    };
    if supplied_raw <= borrowed_raw {
        // Utilization >= 1 would land in region 3 with rates outside the clamp.
        return;
    }

    // Pin starting indices at RAY so the assertions below compare against a known
    // floor. simulate_update_indexes uses `current_timestamp - last_timestamp`
    // for accrual, so feeding (delta_ms, 0) mirrors the clamp above exactly.
    let old_index_raw = Ray::ONE.raw();
    let new_idx = simulate_update_indexes(
        &env,
        delta_ms,
        0,
        Ray::from_raw(borrowed_raw),
        Ray::ONE,
        Ray::from_raw(supplied_raw),
        Ray::ONE,
        &params,
    );

    // Monotonic non-decrease: indices only grow across a positive time step.
    assert!(
        new_idx.borrow_index_ray >= old_index_raw,
        "borrow index regressed: new={} old={} dt={}",
        new_idx.borrow_index_ray,
        old_index_raw,
        delta_ms
    );
    assert!(
        new_idx.supply_index_ray >= old_index_raw,
        "supply index regressed: new={} old={} dt={}",
        new_idx.supply_index_ray,
        old_index_raw,
        delta_ms
    );

    // delta_ms == 0 ⇒ indices are passed through unchanged.
    if delta_ms == 0 {
        assert_eq!(new_idx.borrow_index_ray, old_index_raw);
        assert_eq!(new_idx.supply_index_ray, old_index_raw);
    }
});
