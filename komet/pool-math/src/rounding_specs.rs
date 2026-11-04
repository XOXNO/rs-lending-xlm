//! Directed rounding of the production `Ray` wrappers on their widened path.
//!
//! `Ray::mul_floor` / `mul_ceil` compute `x * y / RAY` and `Ray::div_floor` /
//! `div_ceil` compute `x * RAY / d` through `fp_core::mul_div_floor` /
//! `mul_div_ceil`. `calculate_scaled_supply` and `calculate_scaled_borrow`
//! (ADR-0003: supply shares floor, debt shares ceil) and the floor unscaling
//! helpers are built on them. Every operand here is at least 2^64, so the i128
//! product always overflows and production takes the exact `I256` path; the
//! native i128 fast path (small operands) is outside these claims.
//!
//! Specifications are independent integer inequalities evaluated with `I256`:
//! floor `q*den <= num < (q+1)*den`, ceil `(q-1)*den < num <= q*den`.

use common::{constants::RAY, math::fp::Ray};
use soroban_sdk::{Env, I256};

/// Lower bound of the claimed operand domain (both i128 words nonzero).
pub const WIDE: i128 = 1 << 64;

#[inline(never)]
fn ray_mul_floor(env: &Env, x: i128, y: i128) -> i128 {
    Ray::from(x).mul_floor(env, Ray::from(y)).raw()
}

#[inline(never)]
fn ray_mul_ceil(env: &Env, x: i128, y: i128) -> i128 {
    Ray::from(x).mul_ceil(env, Ray::from(y)).raw()
}

#[inline(never)]
fn ray_div_floor(env: &Env, x: i128, d: i128) -> i128 {
    Ray::from(x).div_floor(env, Ray::from(d)).raw()
}

#[inline(never)]
fn ray_div_ceil(env: &Env, x: i128, d: i128) -> i128 {
    Ray::from(x).div_ceil(env, Ray::from(d)).raw()
}

fn wide(env: &Env, v: i128) -> I256 {
    I256::from_i128(env, v)
}

/// For num >= 0 and den > 0: the floor (or ceil) of num/den fits i128.
fn representable(env: &Env, num: &I256, den: &I256, ceil: bool) -> bool {
    if ceil {
        *num <= wide(env, i128::MAX).mul(den)
    } else {
        *num < wide(env, i128::MAX).add(&wide(env, 1)).mul(den)
    }
}

fn directed(env: &Env, num: &I256, den: &I256, q: i128, ceil: bool) -> bool {
    let quotient = wide(env, q);
    let one = wide(env, 1);
    if ceil {
        quotient.sub(&one).mul(den) < *num && *num <= quotient.mul(den)
    } else {
        quotient.mul(den) <= *num && *num < quotient.add(&one).mul(den)
    }
}

/// x, y >= 2^64 with a representable result: exact floor/ceil of x*y/RAY.
pub fn mul_rounding(env: &Env, x: i128, y: i128, ceil: bool) -> bool {
    if x < WIDE || y < WIDE {
        return true;
    }
    let num = wide(env, x).mul(&wide(env, y));
    let den = wide(env, RAY);
    if !representable(env, &num, &den, ceil) {
        return true;
    }
    let q = if ceil {
        ray_mul_ceil(env, x, y)
    } else {
        ray_mul_floor(env, x, y)
    };
    directed(env, &num, &den, q, ceil)
}

/// x >= 2^64, d > 0 with a representable result: exact floor/ceil of x*RAY/d.
pub fn div_rounding(env: &Env, x: i128, d: i128, ceil: bool) -> bool {
    if x < WIDE || d <= 0 {
        return true;
    }
    let num = wide(env, x).mul(&wide(env, RAY));
    let den = wide(env, d);
    if !representable(env, &num, &den, ceil) {
        return true;
    }
    let q = if ceil {
        ray_div_ceil(env, x, d)
    } else {
        ray_div_floor(env, x, d)
    };
    directed(env, &num, &den, q, ceil)
}

/// Deliberately false: floor of (RAY+1)^2/RAY = RAY+2 (remainder 1), not RAY+3.
pub fn mul_floor_wrong(env: &Env) -> bool {
    ray_mul_floor(env, RAY + 1, RAY + 1) == RAY + 3
}

/// Deliberately false: ceil of (RAY+1)^2/RAY = RAY+3, not RAY+2.
pub fn mul_ceil_wrong(env: &Env) -> bool {
    ray_mul_ceil(env, RAY + 1, RAY + 1) == RAY + 2
}

/// Deliberately false: floor of (2^64)*RAY/3 is not rounded up.
pub fn div_floor_wrong(env: &Env) -> bool {
    let q = ray_div_floor(env, WIDE, 3 * RAY);
    // WIDE / 3 has remainder 1, so floor is WIDE / 3 and ceil is WIDE / 3 + 1.
    q == WIDE / 3 + 1
}

/// Deliberately false: ceil of (2^64)*RAY/(3*RAY) is not truncated.
pub fn div_ceil_wrong(env: &Env) -> bool {
    ray_div_ceil(env, WIDE, 3 * RAY) == WIDE / 3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding_witnesses_cover_exact_and_inexact_cases() {
        let env = Env::default();
        let max_index = 1_000_000_000_000_000_000_000_000_000_000_000_000i128; // 10^36
        for (x, y) in [
            (WIDE, WIDE),
            (RAY, RAY),
            (RAY + 1, RAY + 1),
            (2 * RAY, RAY + 7),
            (max_index, RAY),
            (max_index, 3 * RAY / 2 + 1),
            (i128::MAX / RAY, RAY),
        ] {
            assert!(mul_rounding(&env, x, y, false), "floor {x} {y}");
            assert!(mul_rounding(&env, x, y, true), "ceil {x} {y}");
        }
        for (x, d) in [
            (WIDE, 1),
            (WIDE, 3 * RAY),
            (RAY, RAY),
            (RAY + 1, 3 * RAY),
            (max_index, RAY + 1),
            (i128::MAX, i128::MAX),
            (i128::MAX / 7, 3),
        ] {
            assert!(div_rounding(&env, x, d, false), "floor {x} {d}");
            assert!(div_rounding(&env, x, d, true), "ceil {x} {d}");
        }
        // Inexact witnesses distinguish the two directions.
        assert_eq!(ray_mul_floor(&env, RAY + 1, RAY + 1), RAY + 2);
        assert_eq!(ray_mul_ceil(&env, RAY + 1, RAY + 1), RAY + 3);
        assert_eq!(ray_div_floor(&env, WIDE, 3 * RAY), WIDE / 3);
        assert_eq!(ray_div_ceil(&env, WIDE, 3 * RAY), WIDE / 3 + 1);
        assert!(!mul_floor_wrong(&env));
        assert!(!mul_ceil_wrong(&env));
        assert!(!div_floor_wrong(&env));
        assert!(!div_ceil_wrong(&env));
    }
}
