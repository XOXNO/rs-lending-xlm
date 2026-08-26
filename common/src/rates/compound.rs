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
///
/// # Accuracy
///
/// Every dropped term is positive, so the result is always **below** `e^x`:
/// interest is under-accrued, never over-accrued, which favours the borrower.
/// [`MAX_COMPOUND_DELTA_MS`] plus `MAX_BORROW_RATE_RAY` (2 RAY) bound `x` at 2,
/// where the relative shortfall is 2.37e-4. It falls off fast with the rate:
/// 1.13e-6 at 100% APR, 1.18e-12 at 20%, 2.11e-16 at 5% (all measured over a
/// full one-year chunk). A higher-precision `exp` would cost several times the
/// ~206k CPU instructions this series already spends on `I256` host calls, so
/// the bias is accepted rather than corrected.
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
        sum = sum.checked_add(env, pow.div_by_int(env, divisor));
    }
    sum
}

#[cfg(test)]
#[path = "../../tests/rates/compound.rs"]
mod tests;
