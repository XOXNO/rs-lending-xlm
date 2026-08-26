//! Raw `i128` arithmetic primitives shared by the fixed-point newtypes in
//! [`super::fp`]: overflow-safe `x * y / d` in various rounding modes, plus
//! decimal rescaling between arbitrary decimal precisions.
//!
//! Every multiply-divide first attempts the whole computation in `i128` and
//! only widens the operands to `I256` when the intermediate product does not
//! fit. The widened path is exact, so both paths return the same value; the
//! fast path exists because `I256` arithmetic runs as host calls and costs
//! roughly 15k CPU instructions per operation where native `i128` costs none.

use soroban_sdk::{panic_with_error, Env, I256};

use crate::errors::GenericError;

/// Widens `x`, `y`, and `d` to `I256` for overflow-safe intermediate arithmetic.
fn to_i256_operands(env: &Env, x: i128, y: i128, d: i128) -> (I256, I256, I256) {
    (
        I256::from_i128(env, x),
        I256::from_i128(env, y),
        I256::from_i128(env, d),
    )
}

/// Panics with `GenericError::DivisionByZero` if `d` is zero. Every panicking
/// multiply-divide calls this first so a zero denominator surfaces as a
/// protocol error rather than as an untyped host arithmetic trap.
fn require_nonzero_divisor(env: &Env, d: i128) {
    if d == 0 {
        panic_with_error!(env, GenericError::DivisionByZero);
    }
}

/// Returns true when the exact rational `x * y / d` is strictly negative.
fn quotient_is_negative(x: i128, y: i128, d: i128) -> bool {
    if x == 0 || y == 0 {
        return false;
    }
    ((x < 0) != (y < 0)) != (d < 0)
}

/// Computes `floor(p / d)` in `i128`. Returns `None` only when `d` is zero or
/// the quotient overflows (`i128::MIN / -1`).
fn div_floor_i128(p: i128, d: i128) -> Option<i128> {
    let quotient = p.checked_div(d)?;
    if p % d != 0 && ((p < 0) != (d < 0)) {
        quotient.checked_sub(1)
    } else {
        Some(quotient)
    }
}

/// Computes `ceil(p / d)` in `i128`. Returns `None` only when `d` is zero or
/// the quotient overflows (`i128::MIN / -1`).
fn div_ceil_i128(p: i128, d: i128) -> Option<i128> {
    let quotient = p.checked_div(d)?;
    if p % d != 0 && ((p < 0) == (d < 0)) {
        quotient.checked_add(1)
    } else {
        Some(quotient)
    }
}

/// Computes `floor(p / d)` in `I256`. `rem_euclid` is non-negative, so a
/// nonzero remainder on a negative quotient means truncation landed one above
/// the floor.
///
/// `nonneg` says the caller already proved the quotient cannot be negative,
/// where truncation *is* the floor. Taking that shortcut skips a host
/// `rem_euclid` and two comparisons, which matters because `Ray` products
/// (1e27 x 1e27) always land on this widened path.
fn div_floor_i256(env: &Env, p: &I256, d: &I256, nonneg: bool) -> I256 {
    if nonneg {
        return p.div(d);
    }
    let zero = I256::from_i128(env, 0);
    let quotient = p.div(d);
    if p.rem_euclid(d) != zero && (*p < zero) != (*d < zero) {
        quotient.sub(&I256::from_i128(env, 1))
    } else {
        quotient
    }
}

/// Computes `ceil(p / d)` in `I256`. See [`div_floor_i256`] for `nonneg`.
fn div_ceil_i256(env: &Env, p: &I256, d: &I256, nonneg: bool) -> I256 {
    let zero = I256::from_i128(env, 0);
    let quotient = p.div(d);
    let remainder_is_nonzero = p.rem_euclid(d) != zero;
    let round_up = if nonneg {
        remainder_is_nonzero
    } else {
        remainder_is_nonzero && (*p < zero) == (*d < zero)
    };
    if round_up {
        quotient.add(&I256::from_i128(env, 1))
    } else {
        quotient
    }
}

/// Returns true when the exact rational `x * y / d` cannot be negative.
fn quotient_is_nonnegative(x: i128, y: i128, d: i128) -> bool {
    !quotient_is_negative(x, y, d)
}

/// Computes `x * y / d` rounded half up. Requires `x >= 0`, `y >= 0`, and `d > 0`; a
/// `debug_assert` checks this in debug builds. Panics with `GenericError::DivisionByZero` if
/// `d == 0`, and with `GenericError::MathOverflow` if any other precondition is violated or if
/// the result does not fit in `i128`.
pub fn mul_div_half_up(env: &Env, x: i128, y: i128, d: i128) -> i128 {
    // The zero check runs first so debug and release builds agree on a zero
    // divisor: both surface `DivisionByZero` rather than tripping the assert.
    require_nonzero_divisor(env, d);
    debug_assert!(
        x >= 0 && y >= 0 && d > 0,
        "mul_div_half_up: non-negative x, y and positive d"
    );
    try_mul_div_half_up(env, x, y, d)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
}

/// Computes `x * y / d` rounded half up. Returns `None` if `x < 0`, `y < 0`, `d <= 0`, or the
/// result does not fit in `i128`.
pub fn try_mul_div_half_up(env: &Env, x: i128, y: i128, d: i128) -> Option<i128> {
    if x < 0 || y < 0 || d <= 0 {
        return None;
    }
    let half = d / 2;

    // Fast path: the biased product fits `i128`, so the whole computation is
    // native. `x * y + half` is non-negative here, so `/` is the floor the
    // widened path would produce.
    if let Some(biased) = x
        .checked_mul(y)
        .and_then(|product| product.checked_add(half))
    {
        return Some(biased / d);
    }

    let (x256, y256, d256) = to_i256_operands(env, x, y, d);
    x256.mul(&y256)
        .add(&I256::from_i128(env, half))
        .div(&d256)
        .to_i128()
}

/// Computes `floor(x * y / d)`, rounding toward negative infinity for a negative quotient.
/// Panics with `GenericError::DivisionByZero` if `d == 0`, or with
/// `GenericError::MathOverflow` if the result does not fit in `i128`.
pub fn mul_div_floor(env: &Env, x: i128, y: i128, d: i128) -> i128 {
    require_nonzero_divisor(env, d);
    if let Some(quotient) = x
        .checked_mul(y)
        .and_then(|product| div_floor_i128(product, d))
    {
        return quotient;
    }
    let (x256, y256, d256) = to_i256_operands(env, x, y, d);
    let nonneg = quotient_is_nonnegative(x, y, d);
    to_i128(env, &div_floor_i256(env, &x256.mul(&y256), &d256, nonneg))
}

/// Computes `ceil(x * y / d)`, rounding toward positive infinity for a positive quotient and
/// truncating toward zero for a negative one. Panics with `GenericError::DivisionByZero` if
/// `d == 0`, or with `GenericError::MathOverflow` if the result does not fit in `i128`.
pub fn mul_div_ceil(env: &Env, x: i128, y: i128, d: i128) -> i128 {
    require_nonzero_divisor(env, d);
    if let Some(quotient) = x
        .checked_mul(y)
        .and_then(|product| div_ceil_i128(product, d))
    {
        return quotient;
    }
    let (x256, y256, d256) = to_i256_operands(env, x, y, d);
    let nonneg = quotient_is_nonnegative(x, y, d);
    to_i128(env, &div_ceil_i256(env, &x256.mul(&y256), &d256, nonneg))
}

/// Computes `floor(x * y / d)`, saturating to `i128::MAX` (or `i128::MIN` for a negative
/// quotient) instead of panicking if the result does not fit in `i128`. Panics with
/// `GenericError::DivisionByZero` if `d == 0`.
pub fn mul_div_floor_saturating(env: &Env, x: i128, y: i128, d: i128) -> i128 {
    require_nonzero_divisor(env, d);
    if let Some(quotient) = x
        .checked_mul(y)
        .and_then(|product| div_floor_i128(product, d))
    {
        return quotient;
    }
    let (x256, y256, d256) = to_i256_operands(env, x, y, d);
    div_floor_i256(
        env,
        &x256.mul(&y256),
        &d256,
        quotient_is_nonnegative(x, y, d),
    )
    .to_i128()
    .unwrap_or(if quotient_is_negative(x, y, d) {
        i128::MIN
    } else {
        i128::MAX
    })
}

/// Rescales `a` from `from_decimals` to `to_decimals`, applying `round_down` when the
/// conversion drops digits. Upscaling multiplies by the exact power-of-ten factor and never
/// consults `round_down`; downscaling calls `round_down(a, factor)` with the power of ten
/// being divided out, passing `None` when that power exceeds `i128` — in which case the factor
/// is larger than any `i128`, so `|a| < factor` and each caller knows its own answer without
/// dividing. Returns `a` unchanged when the decimal counts are equal. Panics with
/// `GenericError::MathOverflow` if the upscaling factor or the upscaled value overflows `i128`.
fn rescale(
    env: &Env,
    a: i128,
    from_decimals: u32,
    to_decimals: u32,
    round_down: impl Fn(i128, Option<i128>) -> i128,
) -> i128 {
    if from_decimals == to_decimals {
        return a;
    }
    let factor = 10i128.checked_pow(to_decimals.abs_diff(from_decimals));

    if to_decimals > from_decimals {
        factor
            .and_then(|factor| a.checked_mul(factor))
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
    } else {
        round_down(a, factor)
    }
}

/// Rescales `a` from `from_decimals` to `to_decimals`. Upscaling multiplies by the exact
/// power-of-ten factor; downscaling rounds to the nearest representable value, with exact
/// halves rounding away from zero. Returns `a` unchanged when the decimal counts are equal.
/// Panics with `GenericError::MathOverflow` if the factor or the upscaled value overflows
/// `i128`.
pub fn rescale_half_up(env: &Env, a: i128, from_decimals: u32, to_decimals: u32) -> i128 {
    rescale(env, a, from_decimals, to_decimals, |a, factor| {
        // No representable `a` reaches half of a factor that overflows `i128`.
        let Some(factor) = factor else { return 0 };
        let half = factor / 2;
        if a >= 0 {
            let q = a / factor;
            if a % factor >= half {
                q + 1
            } else {
                q
            }
        } else {
            (a - half) / factor
        }
    })
}

/// Rescales `a` from `from_decimals` to `to_decimals`. Upscaling multiplies by the exact
/// power-of-ten factor; downscaling truncates the quotient toward zero. Returns `a` unchanged
/// when the decimal counts are equal. Panics with `GenericError::MathOverflow` if the factor
/// or the upscaled value overflows `i128`.
pub(crate) fn rescale_floor(env: &Env, a: i128, from_decimals: u32, to_decimals: u32) -> i128 {
    rescale(env, a, from_decimals, to_decimals, |a, factor| {
        // `|a|` is below any factor that overflows `i128`, so truncation gives 0.
        let Some(factor) = factor else { return 0 };
        a / factor
    })
}

/// Rescales `a` from `from_decimals` to `to_decimals`. Upscaling multiplies by the exact
/// power-of-ten factor. Downscaling truncates the quotient toward zero and, for a non-negative
/// `a` with a nonzero remainder, adds 1 to round up; a negative `a` is truncated toward zero
/// without rounding up. Returns `a` unchanged when the decimal counts are equal. Panics with
/// `GenericError::MathOverflow` if the factor or the upscaled value overflows `i128`.
pub(crate) fn rescale_ceil(env: &Env, a: i128, from_decimals: u32, to_decimals: u32) -> i128 {
    rescale(env, a, from_decimals, to_decimals, |a, factor| {
        // `|a|` is below any factor that overflows `i128`: a positive `a` rounds
        // up to 1, anything else truncates to 0.
        let Some(factor) = factor else {
            return i128::from(a > 0);
        };
        let quotient = a / factor;
        if a >= 0 && a % factor != 0 {
            quotient + 1
        } else {
            quotient
        }
    })
}

/// Divides `a` by the positive integer `b`, rounding to the nearest result with exact halves
/// rounding away from zero. Requires `b > 0` in debug builds. Panics with
/// `GenericError::DivisionByZero` if `b` is zero, or with `GenericError::MathOverflow` if
/// adding the rounding half to a non-negative `a` overflows `i128`.
pub fn div_by_int_half_up(env: &Env, a: i128, b: i128) -> i128 {
    require_nonzero_divisor(env, b);
    debug_assert!(b > 0, "div_by_int_half_up expects positive divisor");
    let half_b = b / 2;

    if a >= 0 {
        a.checked_add(half_b)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
            / b
    } else {
        (a - half_b) / b
    }
}

/// Converts an `I256` to `i128`, panicking with `GenericError::MathOverflow` if it does not
/// fit.
fn to_i128(env: &Env, val: &I256) -> i128 {
    val.to_i128()
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
}

#[cfg(test)]
#[path = "../../tests/math/fp_core.rs"]
mod tests;
