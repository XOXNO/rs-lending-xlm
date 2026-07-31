use soroban_sdk::{panic_with_error, Env, I256};

fn to_i256_operands(env: &Env, x: i128, y: i128, d: i128) -> (I256, I256, I256) {
    (
        I256::from_i128(env, x),
        I256::from_i128(env, y),
        I256::from_i128(env, d),
    )
}

pub fn mul_div_half_up(env: &Env, x: i128, y: i128, d: i128) -> i128 {
    debug_assert!(
        x >= 0 && y >= 0 && d > 0,
        "mul_div_half_up: non-negative x, y and positive d"
    );
    try_mul_div_half_up(env, x, y, d)
        .unwrap_or_else(|| panic_with_error!(env, crate::errors::GenericError::MathOverflow))
}

pub fn try_mul_div_half_up(env: &Env, x: i128, y: i128, d: i128) -> Option<i128> {
    if x < 0 || y < 0 || d <= 0 {
        return None;
    }
    let (x256, y256, d256) = to_i256_operands(env, x, y, d);
    let half = d256.div(&I256::from_i128(env, 2));

    let product = x256.mul(&y256).add(&half);
    product.div(&d256).to_i128()
}

pub fn mul_div_floor(env: &Env, x: i128, y: i128, d: i128) -> i128 {
    let (x256, y256, d256) = to_i256_operands(env, x, y, d);
    to_i128(env, &x256.mul(&y256).div(&d256))
}

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

pub fn mul_div_floor_saturating(env: &Env, x: i128, y: i128, d: i128) -> i128 {
    let (x256, y256, d256) = to_i256_operands(env, x, y, d);
    x256.mul(&y256).div(&d256).to_i128().unwrap_or(i128::MAX)
}

fn rescale_upscale(a: i128, diff: u32, factor_msg: &str, value_msg: &str) -> i128 {
    let factor = 10i128.checked_pow(diff).expect(factor_msg);

    a.checked_mul(factor).expect(value_msg)
}

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
    }
}

pub fn rescale_floor(a: i128, from_decimals: u32, to_decimals: u32) -> i128 {
    if from_decimals == to_decimals {
        return a;
    }
    if to_decimals > from_decimals {
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

        a / factor
    }
}

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

        if a >= 0 && remainder != 0 {
            quotient + 1
        } else {
            quotient
        }
    }
}

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

fn to_i128(env: &Env, val: &I256) -> i128 {
    val.to_i128()
        .unwrap_or_else(|| panic_with_error!(env, crate::errors::GenericError::MathOverflow))
}

#[cfg(test)]
#[path = "../../tests/math/fp_core.rs"]
mod tests;
