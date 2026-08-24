//! Raw `i128` arithmetic primitives shared by the fixed-point newtypes in
//! [`super::fp`]: overflow-safe `x * y / d` in various rounding modes, plus
//! decimal rescaling between arbitrary decimal precisions. All multiply-divide
//! operations widen intermediate products to `I256` to avoid `i128` overflow.

use soroban_sdk::{panic_with_error, Env, I256};

/// Widens `x`, `y`, and `d` to `I256` for overflow-safe intermediate arithmetic.
fn to_i256_operands(env: &Env, x: i128, y: i128, d: i128) -> (I256, I256, I256) {
    (
        I256::from_i128(env, x),
        I256::from_i128(env, y),
        I256::from_i128(env, d),
    )
}

/// Computes `x * y / d` rounded half up. Requires `x >= 0`, `y >= 0`, and `d > 0` in debug
/// builds. Panics with `GenericError::MathOverflow` if the result does not fit in `i128`.
pub fn mul_div_half_up(env: &Env, x: i128, y: i128, d: i128) -> i128 {
    debug_assert!(
        x >= 0 && y >= 0 && d > 0,
        "mul_div_half_up: non-negative x, y and positive d"
    );
    try_mul_div_half_up(env, x, y, d)
        .unwrap_or_else(|| panic_with_error!(env, crate::errors::GenericError::MathOverflow))
}

/// Computes `x * y / d` rounded half up. Returns `None` if `x < 0`, `y < 0`, `d <= 0`, or the
/// result does not fit in `i128`.
pub fn try_mul_div_half_up(env: &Env, x: i128, y: i128, d: i128) -> Option<i128> {
    if x < 0 || y < 0 || d <= 0 {
        return None;
    }
    let (x256, y256, d256) = to_i256_operands(env, x, y, d);
    let half = d256.div(&I256::from_i128(env, 2));

    let product = x256.mul(&y256).add(&half);
    product.div(&d256).to_i128()
}

/// Computes `x * y / d`, truncating the quotient toward zero. Does not validate the signs of
/// its inputs. Panics with `GenericError::MathOverflow` if the result does not fit in `i128`.
pub fn mul_div_floor(env: &Env, x: i128, y: i128, d: i128) -> i128 {
    let (x256, y256, d256) = to_i256_operands(env, x, y, d);
    to_i128(env, &x256.mul(&y256).div(&d256))
}

/// Computes `x * y / d`, rounding up when the division leaves a nonzero remainder. The
/// quotient is truncated toward zero, so for a non-negative product this is the mathematical
/// ceiling; for a negative product the result is not the mathematical ceiling. Panics with
/// `GenericError::MathOverflow` if the result does not fit in `i128`.
pub fn mul_div_ceil(env: &Env, x: i128, y: i128, d: i128) -> i128 {
    let (x256, y256, d256) = to_i256_operands(env, x, y, d);
    let product = x256.mul(&y256);
    let quotient = product.div(&d256);
    let remainder = product.rem_euclid(&d256);

    let result = if remainder == I256::from_i128(env, 0) {
        quotient
    } else {
        quotient.add(&I256::from_i128(env, 1))
    };
    to_i128(env, &result)
}

/// Computes `x * y / d`, truncating the quotient toward zero, saturating to `i128::MAX`
/// instead of panicking if the result does not fit in `i128`.
pub fn mul_div_floor_saturating(env: &Env, x: i128, y: i128, d: i128) -> i128 {
    let (x256, y256, d256) = to_i256_operands(env, x, y, d);
    x256.mul(&y256).div(&d256).to_i128().unwrap_or(i128::MAX)
}

/// Rescales `a` from `from_decimals` to `to_decimals`, applying `round_down` when the
/// conversion drops digits. Upscaling multiplies by the exact power-of-ten factor and never
/// consults `round_down`; downscaling calls `round_down(a, factor)` with the power of ten
/// being divided out. Returns `a` unchanged when the decimal counts are equal. Panics if the
/// power-of-ten factor or the upscaled value overflows `i128`.
fn rescale(
    a: i128,
    from_decimals: u32,
    to_decimals: u32,
    round_down: impl Fn(i128, i128) -> i128,
) -> i128 {
    if from_decimals == to_decimals {
        return a;
    }
    let factor = 10i128
        .checked_pow(to_decimals.abs_diff(from_decimals))
        .expect("rescale factor overflow");

    if to_decimals > from_decimals {
        a.checked_mul(factor).expect("rescale upscale overflow")
    } else {
        round_down(a, factor)
    }
}

/// Rescales `a` from `from_decimals` to `to_decimals`. Upscaling multiplies by the exact
/// power-of-ten factor; downscaling rounds to the nearest representable value, with exact
/// halves rounding away from zero. Returns `a` unchanged when the decimal counts are equal.
/// Panics if the power-of-ten factor or the upscaled value overflows `i128`.
pub fn rescale_half_up(a: i128, from_decimals: u32, to_decimals: u32) -> i128 {
    rescale(a, from_decimals, to_decimals, |a, factor| {
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
/// when the decimal counts are equal. Panics if the power-of-ten factor or the upscaled value
/// overflows `i128`.
pub(crate) fn rescale_floor(a: i128, from_decimals: u32, to_decimals: u32) -> i128 {
    rescale(a, from_decimals, to_decimals, |a, factor| a / factor)
}

/// Rescales `a` from `from_decimals` to `to_decimals`. Upscaling multiplies by the exact
/// power-of-ten factor. Downscaling truncates the quotient toward zero and, for a non-negative
/// `a` with a nonzero remainder, adds 1 to round up; a negative `a` is truncated toward zero
/// without rounding up. Returns `a` unchanged when the decimal counts are equal. Panics if the
/// power-of-ten factor or the upscaled value overflows `i128`.
pub(crate) fn rescale_ceil(a: i128, from_decimals: u32, to_decimals: u32) -> i128 {
    rescale(a, from_decimals, to_decimals, |a, factor| {
        let quotient = a / factor;
        if a >= 0 && a % factor != 0 {
            quotient + 1
        } else {
            quotient
        }
    })
}

/// Divides `a` by the positive integer `b`, rounding to the nearest result with exact halves
/// rounding away from zero. Requires `b > 0` in debug builds. Panics if adding the rounding
/// half to a non-negative `a` overflows `i128`.
pub fn div_by_int_half_up(a: i128, b: i128) -> i128 {
    debug_assert!(b > 0, "div_by_int_half_up expects positive divisor");
    let half_b = b / 2;

    if a >= 0 {
        a.checked_add(half_b)
            .expect("div_by_int_half_up rounding overflow")
            / b
    } else {
        (a - half_b) / b
    }
}

/// Converts an `I256` to `i128`, panicking with `GenericError::MathOverflow` if it does not
/// fit.
fn to_i128(env: &Env, val: &I256) -> i128 {
    val.to_i128()
        .unwrap_or_else(|| panic_with_error!(env, crate::errors::GenericError::MathOverflow))
}

#[cfg(test)]
#[path = "../../tests/math/fp_core.rs"]
mod tests;
