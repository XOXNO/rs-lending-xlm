//! Utilization on the widened path: borrowed >= 2^64 (so the i128 product
//! borrowed * RAY overflows and production divides exactly in I256), with
//! borrowed <= supplied. Below 2^64 production uses native i128 division by
//! the symbolic supply (a software division routine); that branch is not
//! covered here. Specification: independent I256 inequalities.

use common::{constants::RAY, math::fp::Ray, rates};
use soroban_sdk::{Env, I256};

/// Lower bound of the claimed borrowed domain.
pub const WIDE: i128 = 1 << 64;

#[inline(never)]
fn utilization(env: &Env, borrowed: i128, supplied: i128) -> i128 {
    rates::utilization(env, Ray::from(borrowed), Ray::from(supplied)).raw()
}

fn in_domain(borrowed: i128, supplied: i128) -> bool {
    borrowed >= WIDE && borrowed <= supplied
}

/// Exact half-up quotient: U*S <= B*RAY + floor(S/2) < (U+1)*S, and 0 <= U <= RAY.
pub fn wide_exact(env: &Env, borrowed: i128, supplied: i128) -> bool {
    if !in_domain(borrowed, supplied) {
        return true;
    }
    let result = utilization(env, borrowed, supplied);
    let s = I256::from_i128(env, supplied);
    let half = s.div(&I256::from_i128(env, 2));
    let numerator = I256::from_i128(env, borrowed)
        .mul(&I256::from_i128(env, RAY))
        .add(&half);
    // All comparisons on the result are I256 (host) comparisons: an i128
    // comparison would make the Wasm branch on the result's 64-bit words.
    let q = I256::from_i128(env, result);
    q >= I256::from_i128(env, 0)
        && q <= I256::from_i128(env, RAY)
        && q.mul(&s) <= numerator
        && numerator < q.add(&I256::from_i128(env, 1)).mul(&s)
}

/// Representative fixed borrowed values for the symbolic-supply claims:
/// one RAY, an odd value near 10^30 and an odd value above 2^100.
pub const FIXED_B: [i128; 3] = [RAY, 1_000_000_000_000_000_000_000_000_007, (1 << 100) + 1];

/// Fixed borrowed value `FIXED_B[k]`, every supplied >= borrowed: the same
/// exact half-up statement as `wide_exact`.
pub fn fixed_b_exact(env: &Env, k: usize, supplied: i128) -> bool {
    wide_exact(env, FIXED_B[k], supplied)
}

/// Bounds only: 0 <= U <= RAY.
pub fn wide_bounds(env: &Env, borrowed: i128, supplied: i128) -> bool {
    if !in_domain(borrowed, supplied) {
        return true;
    }
    let q = I256::from_i128(env, utilization(env, borrowed, supplied));
    q >= I256::from_i128(env, 0) && q <= I256::from_i128(env, RAY)
}

/// Deliberately false: at the exact tie B = 2^64 + 1, S = 2*RAY production
/// rounds up to 2^63 + 1; floor rounding would give 2^63.
pub fn wide_round_wrong(env: &Env) -> bool {
    utilization(env, WIDE + 1, 2 * RAY) == WIDE / 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_witnesses() {
        let env = Env::default();
        for (b, s) in [
            (WIDE, WIDE),
            (WIDE, i128::MAX),
            (WIDE + 1, 2 * RAY),
            (RAY, RAY),
            (RAY, 3 * RAY),
            (i128::MAX, i128::MAX),
            (i128::MAX / 3, i128::MAX),
        ] {
            assert!(wide_exact(&env, b, s), "{b} {s}");
            assert!(wide_bounds(&env, b, s));
        }
        assert_eq!(utilization(&env, WIDE + 1, 2 * RAY), WIDE / 2 + 1);
        assert!(!wide_round_wrong(&env));
        for k in 0..3 {
            for s in [FIXED_B[k], FIXED_B[k] + 1, 2 * FIXED_B[k] + 1, i128::MAX] {
                assert!(fixed_b_exact(&env, k, s));
            }
        }
    }
}
