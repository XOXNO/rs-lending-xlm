//! Raw i128 fixed-point `mul_div` / rescale with I256 intermediates.

use soroban_sdk::{panic_with_error, Env, I256};

fn to_i256_operands(env: &Env, x: i128, y: i128, d: i128) -> (I256, I256, I256) {
    (
        I256::from_i128(env, x),
        I256::from_i128(env, y),
        I256::from_i128(env, d),
    )
}

// Dimensional anchor: D_a{U_a} * D_b{U_b} / D_d{U_d} -> D_{a+b-d}{U_a*U_b/U_d}.
/// Computes `(x * y) / d` with half-up rounding and I256 intermediate.
/// Non-negative inputs only; the `+half` offset rounds toward zero on negatives.
pub fn mul_div_half_up(env: &Env, x: i128, y: i128, d: i128) -> i128 {
    debug_assert!(
        x >= 0 && y >= 0 && d > 0,
        "mul_div_half_up: non-negative x, y and positive d"
    );
    let (x256, y256, d256) = to_i256_operands(env, x, y, d);
    let half = d256.div(&I256::from_i128(env, 2));
    // Rounding offset: half an output ulp expressed in pre-divide integer space.
    let product = x256.mul(&y256).add(&half);
    to_i128(env, &product.div(&d256))
}

/// Computes `(x * y) / d` with floor rounding for non-negative inputs.
pub fn mul_div_floor(env: &Env, x: i128, y: i128, d: i128) -> i128 {
    let (x256, y256, d256) = to_i256_operands(env, x, y, d);
    to_i128(env, &x256.mul(&y256).div(&d256))
}

/// Computes `(x * y) / d` with ceiling rounding for non-negative inputs.
pub fn mul_div_ceil(env: &Env, x: i128, y: i128, d: i128) -> i128 {
    let (x256, y256, d256) = to_i256_operands(env, x, y, d);
    let product = x256.mul(&y256);
    let quotient = product.div(&d256);
    let remainder = product.rem_euclid(&d256);
    // Remainder is only a rounding test; `+1` adds one output raw unit.
    let result = if remainder == I256::from_i128(env, 0) {
        quotient
    } else {
        quotient.add(&I256::from_i128(env, 1))
    };
    to_i128(env, &result)
}

/// Floor `(x * y) / d`; saturates at `i128::MAX` (non-negative inputs).
pub fn mul_div_floor_saturating(env: &Env, x: i128, y: i128, d: i128) -> i128 {
    let (x256, y256, d256) = to_i256_operands(env, x, y, d);
    x256.mul(&y256).div(&d256).to_i128().unwrap_or(i128::MAX)
}

/// Half-up `(x * y) / d`; saturates at `i128::MAX` instead of trapping. Matches
/// `mul_div_half_up` exactly for results that fit `i128` (non-negative inputs).
pub fn mul_div_half_up_saturating(env: &Env, x: i128, y: i128, d: i128) -> i128 {
    let (x256, y256, d256) = to_i256_operands(env, x, y, d);
    let half = d256.div(&I256::from_i128(env, 2));
    let product = x256.mul(&y256).add(&half);
    product.div(&d256).to_i128().unwrap_or(i128::MAX)
}

// Upscale is env-less, so a genuine overflow cannot raise a typed contract
// error; it reverts via the `expect`s. `diff <= RAY_DECIMALS` keeps the factor
// in range, and the value mul overflows only past ~1.7e11 whole tokens of a
// single amount -- an economically unreachable input that reverts correctly.
fn rescale_upscale(a: i128, diff: u32, factor_msg: &str, value_msg: &str) -> i128 {
    let factor = 10i128.checked_pow(diff).expect(factor_msg);
    // D{from}{U} * D{diff}{1} -> D{to}{U}; U is unchanged.
    a.checked_mul(factor).expect(value_msg)
}

// Dimensional anchor: D{from_decimals}{U} -> D{to_decimals}{U}.
/// Rescales between decimal domains with half-up rounding on downscale.
pub fn rescale_half_up(a: i128, from_decimals: u32, to_decimals: u32) -> i128 {
    if from_decimals == to_decimals {
        return a;
    }
    if to_decimals > from_decimals {
        rescale_upscale(
            a,
            to_decimals - from_decimals,
            "rescale_half_up upscale factor overflow",
            "rescale_half_up upscale overflow",
        )
    } else {
        let diff = from_decimals - to_decimals;
        let factor = 10i128
            .checked_pow(diff)
            .expect("rescale_half_up downscale factor overflow");
        let half = factor / 2;
        // Half-up via quotient/remainder, not `(a + half) / factor`: the latter
        // could overflow i128 for a near-`i128::MAX` input, while the result
        // (always <= a on a downscale) fits. D{from}{U} / D{diff}{1} -> D{to}{U}.
        if a >= 0 {
            let q = a / factor;
            if a % factor >= half {
                q + 1
            } else {
                q
            }
        } else {
            // Negatives are rejected upstream; keep round-toward-zero semantics.
            (a - half) / factor
        }
    }
}

/// Rescales and rounds down on downscale for user-credit boundaries.
pub fn rescale_floor(a: i128, from_decimals: u32, to_decimals: u32) -> i128 {
    if from_decimals == to_decimals {
        return a;
    }
    if to_decimals > from_decimals {
        // Upscale: exact, no rounding direction matters.
        rescale_upscale(
            a,
            to_decimals - from_decimals,
            "rescale_floor upscale factor overflow",
            "rescale_floor upscale overflow",
        )
    } else {
        let diff = from_decimals - to_decimals;
        let factor = 10i128
            .checked_pow(diff)
            .expect("rescale_floor downscale factor overflow");
        // D{from}{U} / D{diff}{1} -> D{to}{U}; truncation is directed rounding.
        // Truncation toward zero == floor for non-negative inputs; negatives
        // are rejected upstream.
        a / factor
    }
}

/// Rescales and rounds up on downscale for user-debit boundaries.
pub fn rescale_ceil(a: i128, from_decimals: u32, to_decimals: u32) -> i128 {
    if from_decimals == to_decimals {
        return a;
    }
    if to_decimals > from_decimals {
        rescale_upscale(
            a,
            to_decimals - from_decimals,
            "rescale_ceil upscale factor overflow",
            "rescale_ceil upscale overflow",
        )
    } else {
        let diff = from_decimals - to_decimals;
        let factor = 10i128
            .checked_pow(diff)
            .expect("rescale_ceil downscale factor overflow");
        let quotient = a / factor;
        let remainder = a % factor;
        // D{from}{U} / D{diff}{1} -> D{to}{U}; remainder selects one output ulp.
        // Non-negative input with any sub-ulp remainder rounds up.
        if a >= 0 && remainder != 0 {
            quotient + 1
        } else {
            quotient
        }
    }
}

// Dimensional anchor: Dk{U} / {n} -> Dk{U/n} by caller context.
/// Divides by a positive integer with half-up rounding.
pub fn div_by_int_half_up(a: i128, b: i128) -> i128 {
    debug_assert!(b > 0, "div_by_int_half_up expects positive divisor");
    let half_b = b / 2;
    // Half divisor is a rounding offset for the quotient, not a semantic addend.
    if a >= 0 {
        a.checked_add(half_b)
            .expect("div_by_int_half_up rounding overflow")
            / b
    } else {
        (a - half_b) / b
    }
}

/// Newton iterations allowed before `geometric_mean_floor` gives up. Both seeds
/// below are upper bounds within a small factor of the answer, so convergence is
/// monotone and takes a handful of steps at any imbalance. The cap only bounds
/// CPU against a pathological input; reaching it is a bug, not a valid outcome,
/// so it reverts rather than returning a guess.
const GEOMETRIC_MEAN_MAX_ITERATIONS: u32 = 64;

/// `2^exponent` as an I256, for exponents past what `i128` can hold. Split in
/// half so each factor stays inside `i128` (the caller's exponent is bounded by
/// 128, so each half is at most `2^64`).
fn pow2_i256(env: &Env, exponent: u32) -> I256 {
    // `1i128 << 127` is `i128::MIN`, not a trap, so an out-of-range exponent
    // would silently produce a negative "bound" and break Newton's descent. The
    // sole caller tops out at 128; this keeps that true if another appears.
    debug_assert!(exponent <= 128, "pow2_i256 exponent must stay within 128");
    let low = exponent / 2;
    let high = exponent - low;
    I256::from_i128(env, 1i128 << low).mul(&I256::from_i128(env, 1i128 << high))
}

/// Upper bound on `sqrt(a * b)` from operand bit lengths.
///
/// `a < 2^(ilog2(a)+1)` and likewise for `b`, so `a*b < 2^(ba+bb+2)` and
/// `sqrt(a*b) < 2^((ba+bb)/2 + 1)`. Integer division floors, so one extra
/// exponent keeps the bound sound. Lands within a factor of ~4 of the answer at
/// any imbalance, which is what keeps the iteration count flat.
fn sqrt_upper_bound_pow2(env: &Env, a: i128, b: i128) -> I256 {
    debug_assert!(a > 0 && b > 0, "bit-length seed needs positive operands");
    let exponent = (a.ilog2() + b.ilog2()) / 2 + 2;
    pow2_i256(env, exponent)
}

// Dimensional anchor: Dk{U} * Dk{U} -> Dk{U}; the square root cancels the
// doubled scale, so the result is in the same fixed-point domain as the inputs.
/// Floor of `sqrt(a * b)` for non-negative `a`, `b`, via I256 intermediates.
///
/// The product overflows `i128` well inside real reserve/price ranges (two WAD
/// values near 1e30 give 1e60), so the multiply and every iteration run in I256
/// and only the converged root — which is bounded by `max(a, b)` — narrows back.
///
/// Seeded from the tighter of two upper bounds, never from the product itself:
///
/// * the arithmetic mean, `>= sqrt(a*b)` by AM-GM and *exact* when `a == b`
///   (the balanced-pool case, which then costs zero iterations);
/// * a power of two derived from operand bit lengths, which stays within a small
///   factor of the answer even at extreme imbalance, where the arithmetic mean
///   is useless — for `(1, 8.5e37)` it sits ~62 halvings above the root.
///
/// Both are upper bounds, so the minimum is too, and Newton descends
/// monotonically from it. Seeding from `n` instead would cost one halving per
/// bit, ~127 iterations at the top of the range.
///
/// # Errors
/// * [`GenericError::MathOverflow`] - negative input, or no convergence within
///   [`GEOMETRIC_MEAN_MAX_ITERATIONS`].
pub fn geometric_mean_floor(env: &Env, a: i128, b: i128) -> i128 {
    if a < 0 || b < 0 {
        panic_with_error!(env, crate::errors::GenericError::MathOverflow);
    }
    if a == 0 || b == 0 {
        return 0;
    }

    let one = I256::from_i128(env, 1);
    let two = I256::from_i128(env, 2);
    let n = I256::from_i128(env, a).mul(&I256::from_i128(env, b));

    let arithmetic_mean = I256::from_i128(env, a)
        .add(&I256::from_i128(env, b))
        .div(&two);
    let bit_length_bound = sqrt_upper_bound_pow2(env, a, b);
    let mut x = if arithmetic_mean < bit_length_bound {
        arithmetic_mean
    } else {
        bit_length_bound
    };
    // Both operands are >= 1 here, so both bounds are >= 1 and the division in
    // the first iteration is safe; this only guards the degenerate rounding.
    if x < one {
        x = one.clone();
    }

    // Integer Newton on f(x) = x^2 - n. From an upper-bound seed the sequence is
    // non-increasing until it reaches floor(sqrt(n)), where it either fixes or
    // oscillates up by one; both are caught by the `next >= x` exit.
    let mut iterations = 0u32;
    loop {
        let next = x.add(&n.div(&x)).div(&two);
        if next >= x {
            break;
        }
        x = next;
        iterations += 1;
        if iterations >= GEOMETRIC_MEAN_MAX_ITERATIONS {
            panic_with_error!(env, crate::errors::GenericError::MathOverflow);
        }
    }

    // Newton on integers can settle one above the true floor; step down until
    // `x*x <= n` holds. At most one correction is needed in practice.
    while x.mul(&x) > n {
        x = x.sub(&one);
    }

    to_i128(env, &x)
}

fn to_i128(env: &Env, val: &I256) -> i128 {
    val.to_i128()
        .unwrap_or_else(|| panic_with_error!(env, crate::errors::GenericError::MathOverflow))
}

#[cfg(test)]
#[path = "../../tests/math/fp_core.rs"]
mod tests;
