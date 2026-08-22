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

    let mut sum = Ray::ONE.checked_add(env, x);
    let mut pow = x;
    for divisor in [2, 6, 24, 120, 720, 5_040, 40_320] {
        pow = pow.mul(env, x);
        sum = sum.checked_add(env, pow.div_by_int(divisor));
    }
    sum
}

#[cfg(test)]
#[path = "../../tests/rates/compound.rs"]
mod tests;
