//! Independent quotient inequalities for production arithmetic.
//!
//! Returning true outside a claim's stated domain is a premise, not evidence
//! that the excluded zero-divisor or overflow error was handled correctly.
use common::{
    constants::{RAY, RAY_DECIMALS},
    math::{fp::Ray, fp_core},
    rates,
};
use soroban_sdk::{Env, I256};

// I256 avoids abs(i128::MIN). Products of i128 operands and all q +/- 1
// boundary products below fit I256, including a denominator of 2^127.
fn ratio(env: &Env, x: i128, y: i128, divisor: i128) -> (I256, I256) {
    let numerator = I256::from_i128(env, x).mul(&I256::from_i128(env, y));
    let denominator = I256::from_i128(env, divisor);
    if divisor < 0 {
        let zero = I256::from_i128(env, 0);
        (zero.sub(&numerator), zero.sub(&denominator))
    } else {
        (numerator, denominator)
    }
}

fn representable(env: &Env, numerator: &I256, denominator: &I256, ceil: bool) -> bool {
    let min = I256::from_i128(env, i128::MIN);
    let max = I256::from_i128(env, i128::MAX);
    let one = I256::from_i128(env, 1);
    if ceil {
        min.sub(&one).mul(denominator) < *numerator && *numerator <= max.mul(denominator)
    } else {
        min.mul(denominator) <= *numerator && *numerator < max.add(&one).mul(denominator)
    }
}

fn quotient_matches(env: &Env, numerator: &I256, denominator: &I256, q: i128, ceil: bool) -> bool {
    let quotient = I256::from_i128(env, q);
    let one = I256::from_i128(env, 1);
    if ceil {
        quotient.sub(&one).mul(denominator) < *numerator && *numerator <= quotient.mul(denominator)
    } else {
        quotient.mul(denominator) <= *numerator && *numerator < quotient.add(&one).mul(denominator)
    }
}

fn round_exact(env: &Env, x: i128, y: i128, divisor: i128, ceil: bool) -> bool {
    if divisor == 0 {
        return true;
    }
    let (numerator, denominator) = ratio(env, x, y, divisor);
    if !representable(env, &numerator, &denominator, ceil) {
        return true;
    }
    let result = if ceil {
        fp_core::mul_div_ceil(env, x, y, divisor)
    } else {
        fp_core::mul_div_floor(env, x, y, divisor)
    };
    quotient_matches(env, &numerator, &denominator, result, ceil)
}

/// Every signed input with nonzero divisor and representable mathematical floor.
pub fn floor_exact(env: &Env, x: i128, y: i128, divisor: i128) -> bool {
    round_exact(env, x, y, divisor, false)
}

/// Every signed input with nonzero divisor and representable mathematical ceil.
pub fn ceil_exact(env: &Env, x: i128, y: i128, divisor: i128) -> bool {
    round_exact(env, x, y, divisor, true)
}

/// Every signed input with nonzero divisor, including both overflow directions.
pub fn floor_saturating_exact(env: &Env, x: i128, y: i128, divisor: i128) -> bool {
    if divisor == 0 {
        return true;
    }
    let (numerator, denominator) = ratio(env, x, y, divisor);
    let result = fp_core::mul_div_floor_saturating(env, x, y, divisor);
    let min = I256::from_i128(env, i128::MIN);
    let first_above_max = I256::from_i128(env, i128::MAX).add(&I256::from_i128(env, 1));
    if numerator < min.mul(&denominator) {
        result == i128::MIN
    } else if numerator >= first_above_max.mul(&denominator) {
        result == i128::MAX
    } else {
        quotient_matches(env, &numerator, &denominator, result, false)
    }
}

fn scaled_exact(env: &Env, amount: i128, decimals: u32, index: i128, ceil: bool) -> bool {
    if amount < 0 || index <= 0 || decimals > RAY_DECIMALS {
        return true;
    }
    let factor = 10i128.pow(RAY_DECIMALS - decimals);
    let rescaled = I256::from_i128(env, amount).mul(&I256::from_i128(env, factor));
    if rescaled > I256::from_i128(env, i128::MAX) {
        return true;
    }
    // Check the rescale intermediate before multiplying again: the unchecked
    // three-factor product of arbitrary inputs need not fit I256.
    let numerator = rescaled.mul(&I256::from_i128(env, RAY));
    let denominator = I256::from_i128(env, index);
    if !representable(env, &numerator, &denominator, ceil) {
        return true;
    }
    let result = if ceil {
        rates::calculate_scaled_borrow(env, amount, decimals, Ray::from(index)).raw()
    } else {
        rates::calculate_scaled_supply(env, amount, decimals, Ray::from(index)).raw()
    };
    quotient_matches(env, &numerator, &denominator, result, ceil)
}

/// Nonnegative amount, positive index, decimals <= 27; rescale and floor fit i128.
pub fn scaled_supply_exact(env: &Env, amount: i128, decimals: u32, index: i128) -> bool {
    scaled_exact(env, amount, decimals, index, false)
}

/// Nonnegative amount, positive index, decimals <= 27; rescale and ceil fit i128.
pub fn scaled_borrow_exact(env: &Env, amount: i128, decimals: u32, index: i128) -> bool {
    scaled_exact(env, amount, decimals, index, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonvacuous_signed_and_scaled_witnesses() {
        let env = Env::default();
        for (x, y, d, floor, ceil) in [
            (7, 1, 3, 2, 3),
            (-7, 1, 3, -3, -2),
            (7, 1, -3, -3, -2),
            (-7, 1, -3, 2, 3),
            (i128::MIN, 1, 1, i128::MIN, i128::MIN),
            (i128::MAX, 1, 1, i128::MAX, i128::MAX),
            (i128::MIN, 1, i128::MIN, 1, 1),
            (1, 1, i128::MIN, -1, 0),
            (RAY + 1, RAY + 1, RAY, RAY + 2, RAY + 3),
            (-(RAY + 1), RAY + 1, RAY, -RAY - 3, -RAY - 2),
        ] {
            let (n, denominator) = ratio(&env, x, y, d);
            assert!(representable(&env, &n, &denominator, false));
            assert!(representable(&env, &n, &denominator, true));
            let wrong_floor = if floor == i128::MAX {
                floor - 1
            } else {
                floor + 1
            };
            assert!(!quotient_matches(
                &env,
                &n,
                &denominator,
                wrong_floor,
                false
            ));
            let wrong_ceil = if ceil == i128::MIN {
                ceil + 1
            } else {
                ceil - 1
            };
            assert!(!quotient_matches(&env, &n, &denominator, wrong_ceil, true));
            assert_eq!(fp_core::mul_div_floor(&env, x, y, d), floor);
            assert_eq!(fp_core::mul_div_ceil(&env, x, y, d), ceil);
            assert!(floor_exact(&env, x, y, d));
            assert!(ceil_exact(&env, x, y, d));
            assert!(floor_saturating_exact(&env, x, y, d));
        }
        for (x, y, d, expected) in [
            (i128::MIN, 1, -1, i128::MAX),
            (i128::MIN, 2, 1, i128::MIN),
            (i128::MAX, i128::MAX, 1, i128::MAX),
        ] {
            assert_eq!(fp_core::mul_div_floor_saturating(&env, x, y, d), expected);
            assert!(floor_saturating_exact(&env, x, y, d));
        }
        for (amount, decimals, index, floor, ceil) in [
            (0, 7, RAY, 0, 0),
            (
                1,
                7,
                RAY,
                100_000_000_000_000_000_000,
                100_000_000_000_000_000_000,
            ),
            (1, 18, 3 * RAY, 333_333_333, 333_333_334),
            (1, 27, 3 * RAY, 0, 1),
            (1, 26, 3 * RAY, 3, 4),
            (RAY + 1, 27, RAY, RAY + 1, RAY + 1),
        ] {
            assert_eq!(
                rates::calculate_scaled_supply(&env, amount, decimals, Ray::from(index)).raw(),
                floor
            );
            assert_eq!(
                rates::calculate_scaled_borrow(&env, amount, decimals, Ray::from(index)).raw(),
                ceil
            );
            assert!(scaled_supply_exact(&env, amount, decimals, index));
            assert!(scaled_borrow_exact(&env, amount, decimals, index));
        }
    }
}
