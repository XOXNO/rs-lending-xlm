//! Computes compounding interest growth factors from a per-millisecond rate
//! using a truncated Taylor-series expansion of `e^x`.

use soroban_sdk::{panic_with_error, Env, I256};

use crate::constants::MILLISECONDS_PER_YEAR;
use crate::math::fp::Ray;

/// Largest time delta, in milliseconds, accepted by a single call to
/// [`compound_interest`]. Equal to one year in milliseconds.
/// `simulate_update_indexes_body` splits longer intervals into chunks no
/// larger than this before compounding each chunk.
pub const MAX_COMPOUND_DELTA_MS: u64 = MILLISECONDS_PER_YEAR;

/// Computes the compounding growth factor for `rate` applied over `delta_ms`
/// milliseconds, approximating `e^(rate * delta_ms)`.
///
/// Returns `Ray::ONE` when `delta_ms` is zero. Otherwise scales `rate` by
/// `delta_ms` and sums a Taylor series through the eighth-order term. Panics
/// if the scaled exponent does not fit in `i128`.
pub fn compound_interest(env: &Env, rate: Ray, delta_ms: u64) -> Ray {
    if delta_ms == 0 {
        return Ray::ONE;
    }

    let x = Ray::from({
        let r = I256::from_i128(env, rate.raw());
        let d = I256::from_i128(env, delta_ms as i128);
        r.mul(&d)
            .to_i128()
            .unwrap_or_else(|| panic_with_error!(env, crate::errors::GenericError::MathOverflow))
    });

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
