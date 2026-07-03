use soroban_sdk::{panic_with_error, Env, I256};

// Widens the three operands to I256 for an overflow-safe `mul_div`.
fn to_i256_operands(env: &Env, x: i128, y: i128, d: i128) -> (I256, I256, I256) {
    (
        I256::from_i128(env, x),
        I256::from_i128(env, y),
        I256::from_i128(env, d),
    )
}

// Dimensional anchor: D_a{U_a} * D_b{U_b} / D_d{U_d} -> D_{a+b-d}{U_a*U_b/U_d}.
/// Computes `(x * y) / d` with half-up rounding and I256 intermediate.
pub fn mul_div_half_up(env: &Env, x: i128, y: i128, d: i128) -> i128 {
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

/// Computes signed `(x * y) / d` with half-up rounding away from zero.
pub fn mul_div_half_up_signed(env: &Env, x: i128, y: i128, d: i128) -> i128 {
    let (x256, y256, d256) = to_i256_operands(env, x, y, d);
    let half = d256.div(&I256::from_i128(env, 2));
    let product = x256.mul(&y256);

    // Signed half-up uses the same pre-divide rounding offset away from zero.
    let rounded = if product < I256::from_i128(env, 0) {
        product.sub(&half)
    } else {
        product.add(&half)
    };
    to_i128(env, &rounded.div(&d256))
}

// Upscales `a` by `10^diff`, mapping both overflow points to caller-supplied
// messages via `checked_pow`/`checked_mul` rather than a generic panic.
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
        // D{from}{U} / D{diff}{1} -> D{to}{U}; `half` only rounds scale loss.
        if a >= 0 {
            a.checked_add(half)
                .expect("rescale_half_up rounding overflow")
                / factor
        } else {
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

fn to_i128(env: &Env, val: &I256) -> i128 {
    val.to_i128()
        .unwrap_or_else(|| panic_with_error!(env, crate::errors::GenericError::MathOverflow))
}

#[cfg(test)]
#[path = "../../tests/math/fp_core.rs"]
mod tests;
