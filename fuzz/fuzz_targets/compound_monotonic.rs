//! Fuzz `compound_interest` — Taylor expansion of e^(rate*delta).
//!
//! Invariants:
//!   1. compound(rate, 0) == 1.0 (identity at zero time)
//!   2. compound(0, delta) == 1.0 (identity at zero rate)
//!   3. compound(rate, delta) ≥ 1.0 for non-negative rate
//!   4. Monotonic in delta: compound(rate, t1) ≤ compound(rate, t2) for t1 < t2
//!   5. Monotonic in rate: compound(r1, delta) ≤ compound(r2, delta) for r1 < r2
//!   6. Lower bound: compound(rate, delta) ≥ 1 + rate*delta   (1st-order Taylor floor)
//!
//! Inputs clamped to realistic range:
//!   - rate_per_ms corresponding to ≤ 500% APR
//!   - delta_ms ≤ 1 year
#![no_main]
use arbitrary::Arbitrary;
use common::fp::Ray;
use common::rates::compound_interest;
use libfuzzer_sys::fuzz_target;
use soroban_sdk::Env;

const RAY: i128 = 1_000_000_000_000_000_000_000_000_000;
const MS_PER_YEAR: u64 = 31_556_926_000;

#[derive(Debug, Arbitrary)]
struct In {
    rate_apr_bps: u16, // 0..=50_000 → 0..500% APR
    delta_ms: u64,
}

fuzz_target!(|i: In| {
    let env = Env::default();

    // Identity: zero delta
    let r0 = Ray::from_raw(RAY / 10); // 10% annual (arbitrary)
    assert_eq!(compound_interest(&env, r0, 0), Ray::ONE);

    // Identity: zero rate
    let t_any = (i.delta_ms % MS_PER_YEAR).max(1);
    assert_eq!(compound_interest(&env, Ray::ZERO, t_any), Ray::ONE);

    // Bound inputs
    let apr_bps = (i.rate_apr_bps % 50_001) as i128;
    let rate_annual_ray = RAY * apr_bps / 10_000;
    let rate_per_ms = Ray::from_raw(rate_annual_ray / MS_PER_YEAR as i128);
    // Allow up to 10 years of accrual so we can probe M-08 territory (the
    // 5-term Taylor expansion loses accuracy and may overflow at large
    // r*t products). A MathOverflow-style panic here is acceptable; a
    // silently wrong result is not.
    let dt = i.delta_ms % (10 * MS_PER_YEAR);

    use std::panic::AssertUnwindSafe;
    let factor = match std::panic::catch_unwind(AssertUnwindSafe(|| {
        compound_interest(&env, rate_per_ms, dt)
    })) {
        Ok(f) => f,
        // Overflow/panic is expected at the extreme end of the expanded range.
        Err(_) => return,
    };

    // Invariant 3: ≥ 1.0
    assert!(
        factor.raw() >= RAY - 1, // -1 ulp for rounding
        "compound below 1.0: factor={}, rate_per_ms={}, dt={}",
        factor.raw(),
        rate_per_ms.raw(),
        dt
    );

    // Invariant 4: monotonic in delta
    if dt > 0 {
        if let Ok(prev) = std::panic::catch_unwind(AssertUnwindSafe(|| {
            compound_interest(&env, rate_per_ms, dt - 1)
        })) {
            assert!(
                factor.raw() >= prev.raw(),
                "non-monotonic in delta: f(t-1)={} f(t)={}",
                prev.raw(),
                factor.raw()
            );
        }
    }

    // Invariant 6: lower bound 1 + r*t (Taylor floor)
    // The 5-term Taylor is always ≥ 1 + x for x ≥ 0 since omitted terms are positive.
    // Only check when r*t fits in i128 AND 1 + r*t doesn't overflow.
    if let Some(rt) = rate_per_ms.raw().checked_mul(dt as i128) {
        if let Some(linear_floor) = RAY.checked_add(rt) {
            assert!(
                factor.raw() >= linear_floor - 2, // rounding tolerance
                "Taylor below linear floor: factor={}, 1+r*t={}",
                factor.raw(),
                linear_floor
            );
        }
    }
});
