//! Interest pipeline: borrow rate, compound factor, supplier/fee split, index update.
//!
//! Parameters are generated inside the production-valid domain and gated by the
//! exact `MarketParamsRaw::verify` the pool runs at market creation. Because that
//! caps `max_borrow_rate` at `MAX_BORROW_RATE_RAY` (2 RAY), every per-chunk
//! `compound_interest` evaluation stays inside the Taylor-accurate domain
//! (x = rate·chunk ≤ 2 RAY) without any ad-hoc rate·dt clamp.
#![no_main]
use arbitrary::Arbitrary;
use common::constants::{BPS, MAX_BORROW_RATE_RAY, MILLISECONDS_PER_YEAR, RAY};
use common::math::fp::Ray;
use common::rates::{
    calculate_borrow_rate, calculate_deposit_rate, calculate_supplier_rewards, compound_interest,
    simulate_update_indexes, MAX_COMPOUND_DELTA_MS,
};
use common::types::{MarketParams, MarketParamsRaw, PoolStateRaw, PoolSyncData};
use libfuzzer_sys::fuzz_target;
use soroban_sdk::{Address, Env};

const MS_PER_YEAR: u64 = MILLISECONDS_PER_YEAR;

/// Longest accrual gap simulated: 10 years. Mirrors a market left untouched
/// across (extreme) Soroban storage-TTL renewals and bounds the chunk loop in
/// `simulate_update_indexes` to ≤10 iterations. With the verified 2-RAY rate
/// cap, the borrow index compounds at most e^2≈7.4× per yearly chunk, so even
/// 10 chunks stay well inside i128.
const MAX_ACCRUAL_MS: u64 = 10 * MS_PER_YEAR;

/// Magnitude bound on fuzzed pool amounts (≈ a token's finite scaled supply),
/// keeping every `amount · index` product inside i128 after 10 years of
/// compounding. Not a domain clamp.
const AMOUNT_CAP_RAW: i128 = 100_000_000_000_000_000; // 1e17

/// Starting borrow index spans [RAY, RAY + 9·RAY] = [1×, 10×]. `borrow_index_units`
/// (u64, max ≈1.8e19) is smaller than RAY, so it must be scaled up, not modulo'd.
/// Pre-dividing the 9·RAY span by u64::MAX keeps the multiply inside i128.
const START_BORROW_INDEX_GROWTH: i128 = 9 * RAY;
const BI_SCALE: i128 = START_BORROW_INDEX_GROWTH / (u64::MAX as i128);

/// Floor for the starting supply index, as `borrow_index / 16`. Supply index
/// sits at or below the borrow index (suppliers earn a fraction) but never far
/// below: bounding `borrow/supply ≤ 16` at the start — and the supply index only
/// grows during accrual — keeps `borrow/supply ≤ MAX_BORROW_INDEX/(RAY/16)`, so
/// utilization stays inside i128 across the full 10-year accrual.
const SUPPLY_INDEX_MIN_DIVISOR: i128 = 16;

/// Leading fields drive the rate-model geometry; the tail carries accrual inputs
/// and the fuzzed starting indices. Appended fields keep the original prefix
/// offsets so existing seeds still decode.
#[derive(Debug, Arbitrary)]
struct In {
    // --- rate-model geometry (mapped into the verified domain) ---
    util_bps: u16,
    base_pct: u8,
    s1_pct: u8,
    s2_pct: u8,
    s3_pct: u16,
    mid_pct: u8,
    opt_pct: u8,
    max_pct: u16,
    max_util_pct: u8,
    reserve_pct: u8,
    // --- accrual (original prefix offsets preserved; new fields appended) ---
    delta_ms: u64, // total accrual gap (multi-chunk)
    borrowed_units: u64,
    supplied_units: u64,
    chunk_units: u64,        // per-chunk compound delta, decorrelated from delta_ms
    borrow_index_units: u64, // starting borrow index in [RAY, 10·RAY]
    supply_index_units: u64, // starting supply index in [borrow_index/16, borrow_index]
}

/// Builds a `MarketParamsRaw` that satisfies `InterestRateModel::verify` by
/// construction: monotone `0 ≤ base ≤ s1 ≤ s2 ≤ s3 ≤ max ≤ MAX_BORROW_RATE_RAY`
/// with `max > base`, breakpoints `0 < mid < optimal < RAY ≤ … `, `optimal ≤
/// max_util ≤ RAY`, and `reserve_factor < BPS`.
fn make_params(env: &Env, i: &In) -> MarketParamsRaw {
    let cap = MAX_BORROW_RATE_RAY; // 2 RAY — production ceiling on the borrow rate.

    // Cumulative non-decreasing slopes within [0, cap]. Each fraction divides by
    // its input's field width so it spans the full remaining headroom — the
    // verified slope range, not just its low end. `base` stays modest (≤ ~cap/4)
    // so the slopes have room to vary across a realistic curve.
    let base = cap * (i.base_pct as i128) / 1_024; // [0, ~cap/4]
    let s1 = base + (cap - base) * (i.s1_pct as i128) / 256; // u8 → [base, ~cap)
    let s2 = s1 + (cap - s1) * (i.s2_pct as i128) / 256; // u8 → [s1, ~cap)
    let s3 = s2 + (cap - s2) * (i.s3_pct as i128) / 65_536; // u16 → [s2, ~cap)
                                                            // max ∈ [s3, cap), forced strictly above base.
    let max_rate = (s3 + (cap - s3) * (i.max_pct as i128) / 65_536).max(base + 1); // u16

    // Utilization breakpoints: 0 < mid < optimal < RAY, optimal ≤ max_util ≤ RAY.
    let mid = RAY * (i.mid_pct as i128 % 98 + 1) / 100; // [1%, 98%] of RAY
    let optimal = mid + (RAY - mid) * (i.opt_pct as i128 % 99 + 1) / 101; // (mid, RAY)
    let max_util = optimal + (RAY - optimal) * (i.max_util_pct as i128) / 256; // [optimal, RAY)

    MarketParamsRaw {
        max_borrow_rate: max_rate,
        base_borrow_rate: base,
        slope1: s1,
        slope2: s2,
        slope3: s3,
        mid_utilization: mid,
        optimal_utilization: optimal,
        max_utilization: max_util,
        // `reserve_pct % BPS` would be a no-op (u8 max 255). Scale the byte
        // across the full verified range [0, BPS), hitting the BPS-1 boundary.
        reserve_factor: (i.reserve_pct as u32) * (BPS as u32 - 1) / (u8::MAX as u32),
        supply_cap: 0,
        borrow_cap: 0,
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
        params.mid_utilization.raw()
    );

    let max_per_ms = params.max_borrow_rate.raw() / MS_PER_YEAR as i128;
    assert!(
        rate.raw() <= max_per_ms + 2,
        "rate exceeded max: rate={} max_per_ms={}",
        rate.raw(),
        max_per_ms
    );

    // Monotonically non-decreasing in utilization (2 ulp slack across joins).
    if util_bps < 10_000 {
        let util_hi = Ray::from(RAY * (util_bps + 1) / 10_000);
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
    if params.reserve_factor.raw() == 0 {
        assert_eq!(fee.raw(), 0, "fee non-zero with reserve_factor=0");
    }

    // Half-up bound on Bps::apply_to: |fee*BPS - rf*accrued| ≤ BPS/2 + 1.
    if let (Some(fee_scaled), Some(rf_scaled)) = (
        fee.raw().checked_mul(BPS),
        params.reserve_factor.raw().checked_mul(accrued.raw()),
    ) {
        let err = (fee_scaled - rf_scaled).abs();
        assert!(
            err <= BPS / 2 + 1,
            "§5 fee rounding: fee*BPS={} rf*accrued={} err={} rf={}",
            fee_scaled,
            rf_scaled,
            err,
            params.reserve_factor.raw()
        );
    }
}

fuzz_target!(|i: In| {
    let env = Env::default();

    // Build params in the production-valid domain, then gate them through the
    // exact check `create_market` runs. Construction keeps them valid, so this
    // is a self-check that must never panic — if it does, the generator drifted
    // from `InterestRateModel::verify`.
    let params_raw = make_params(&env, &i);
    params_raw.verify(&env);
    let params = MarketParams::from(&params_raw);

    // ---- per-chunk rate curve at an arbitrary utilization point ----
    let util_bps = (i.util_bps % 10_001) as i128;
    let util = Ray::from(RAY * util_bps / 10_000);
    let rate = calculate_borrow_rate(&env, util, &params);
    assert_rate_invariants(&env, util_bps, &params, rate);

    // Production evaluates `compound_interest` per accrual chunk of at most
    // MAX_COMPOUND_DELTA_MS. With the verified cap (rate ≤ 2 RAY/yr), one chunk
    // keeps x = rate·chunk ≤ 2 RAY, inside the Taylor-accurate domain.
    // Decorrelated from total_delta_ms below (its own entropy field).
    let chunk_ms = i.chunk_units % (MAX_COMPOUND_DELTA_MS + 1); // [0, MAX_COMPOUND_DELTA_MS]
    let factor = compound_interest(&env, rate, chunk_ms);
    assert_compound_invariants(&env, rate, chunk_ms, factor);

    // §5 split at this chunk: old_index pinned at RAY so new_index = factor.
    let borrowed_raw = (i.borrowed_units as i128 % AMOUNT_CAP_RAW) + 1; // [1, AMOUNT_CAP_RAW]
    assert_interest_split(&env, &params, Ray::from(borrowed_raw), factor, Ray::ONE);

    // ---- calculate_deposit_rate (per chunk) ----
    let deposit_rate = calculate_deposit_rate(&env, util, rate, params.reserve_factor);
    assert!(
        deposit_rate.raw() >= 0,
        "negative deposit rate: {}",
        deposit_rate.raw()
    );
    // Supplier share is (1 - reserve_factor) × utilization × borrow_rate ≤ borrow_rate.
    assert!(
        deposit_rate.raw() <= rate.raw() + 1,
        "deposit rate > borrow rate: dep={} bor={}",
        deposit_rate.raw(),
        rate.raw()
    );
    if params.reserve_factor.raw() == 0 && util.raw() > 0 {
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

    // ---- simulate_update_indexes (real read-path accrual) ----
    // Fuzz a starting state with supplied > borrowed (util < 1, as production
    // markets enter below max_utilization). The verified 2-RAY rate cap bounds
    // every internal chunk's rate, so multi-year accrual compounds without the
    // index overflow an unbounded max_borrow_rate would cause — no domain
    // clamp, no util-reproducing supplied derivation, no single-chunk bound.
    let supplied_raw = borrowed_raw + 1 + (i.supplied_units as i128 % AMOUNT_CAP_RAW);
    let total_delta_ms = i.delta_ms % (MAX_ACCRUAL_MS + 1); // [0, 10 years]

    // Fuzz the starting indices, not always RAY: production calls
    // simulate_update_indexes from whatever the current indices are. Scale the
    // u64 entropy across the safe ranges (pre-dividing each span by u64::MAX so
    // the multiply can't overflow); the bounds keep utilization inside i128
    // across the accrual (see the constants above).
    let start_borrow_index = RAY + i.borrow_index_units as i128 * BI_SCALE; // [RAY, 10·RAY]
    let si_floor = start_borrow_index / SUPPLY_INDEX_MIN_DIVISOR;
    let si_scale = (start_borrow_index - si_floor) / (u64::MAX as i128);
    let start_supply_index = si_floor + i.supply_index_units as i128 * si_scale; // [BI/16, BI]

    let sync = PoolSyncData {
        params: params_raw,
        state: PoolStateRaw {
            supplied: supplied_raw,
            borrowed: borrowed_raw,
            revenue: 0,
            cash: 0,
            borrow_index: start_borrow_index,
            supply_index: start_supply_index,
            last_timestamp: 0,
        },
    };
    let new_idx = simulate_update_indexes(&env, total_delta_ms, &sync);

    // Monotonic non-decrease from the starting indices: accrual only grows them.
    assert!(
        new_idx.borrow_index.raw() >= start_borrow_index,
        "borrow index regressed: new={} old={} dt={}",
        new_idx.borrow_index.raw(),
        start_borrow_index,
        total_delta_ms
    );
    assert!(
        new_idx.supply_index.raw() >= start_supply_index,
        "supply index regressed: new={} old={} dt={}",
        new_idx.supply_index.raw(),
        start_supply_index,
        total_delta_ms
    );

    // total_delta_ms == 0 ⇒ indices are passed through unchanged.
    if total_delta_ms == 0 {
        assert_eq!(new_idx.borrow_index.raw(), start_borrow_index);
        assert_eq!(new_idx.supply_index.raw(), start_supply_index);
    }
});
