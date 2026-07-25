//! Compound-interest factor for one accrual chunk.
//!
//! Callers must chunk elapsed time at [`MAX_COMPOUND_DELTA_MS`]; the truncated
//! Taylor expansion is only accurate inside that bound.

use soroban_sdk::{panic_with_error, Env, I256};

use crate::constants::MILLISECONDS_PER_YEAR;
use crate::math::fp::Ray;

/// Max compound-interest chunk (one year in ms).
pub const MAX_COMPOUND_DELTA_MS: u64 = MILLISECONDS_PER_YEAR;

/// Compound interest factor `e^(rate * delta_ms)` over one accrual chunk.
///
/// The caller must chunk `delta_ms` at `MAX_COMPOUND_DELTA_MS`; the truncated
/// expansion is only accurate inside that bound.
pub fn compound_interest(env: &Env, rate: Ray, delta_ms: u64) -> Ray {
    if delta_ms == 0 {
        return Ray::ONE;
    }

    // dimensional: Ray<RatePerMs> * TimeMs -> Ray<1>; I256 guards extreme products.
    let x = Ray::from({
        let r = I256::from_i128(env, rate.raw());
        let d = I256::from_i128(env, delta_ms as i128);
        r.mul(&d)
            .to_i128()
            .unwrap_or_else(|| panic_with_error!(env, crate::errors::GenericError::MathOverflow))
    });

    // 8-term Taylor expansion of e^x. Remainder R8(x) ≤ x^9 / 9! → ≈ 0.14%
    // absolute error at x = 2. Per-chunk x is bounded by the accrual loop.
    //
    // Written flat on purpose. Each power is built with half-up multiplies and
    // divided by k! exactly once; folding this into a loop over
    // `term = term * x / k` divides at every step and rounds differently, which
    // moves the resulting index. Formal rules assert exact index values, so this
    // is a rounding contract, not a style choice.
    let x_sq = x.mul(env, x);
    let x_cub = x_sq.mul(env, x);
    let x_pow4 = x_cub.mul(env, x);
    let x_pow5 = x_pow4.mul(env, x);
    let x_pow6 = x_pow5.mul(env, x);
    let x_pow7 = x_pow6.mul(env, x);
    let x_pow8 = x_pow7.mul(env, x);

    let term2 = x_sq.div_by_int(2);
    let term3 = x_cub.div_by_int(6);
    let term4 = x_pow4.div_by_int(24);
    let term5 = x_pow5.div_by_int(120);
    let term6 = x_pow6.div_by_int(720);
    let term7 = x_pow7.div_by_int(5_040);
    let term8 = x_pow8.div_by_int(40_320);

    let mut sum = Ray::ONE;
    for term in [x, term2, term3, term4, term5, term6, term7, term8] {
        sum = sum.checked_add(env, term);
    }
    sum
}

#[cfg(test)]
#[path = "../../tests/rates/compound.rs"]
mod tests;
