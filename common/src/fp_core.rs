use soroban_sdk::{panic_with_error, Env, I256};

// (x * y + d/2) / d.
pub fn mul_div_half_up(env: &Env, x: i128, y: i128, d: i128) -> i128 {
    let x256 = I256::from_i128(env, x);
    let y256 = I256::from_i128(env, y);
    let d256 = I256::from_i128(env, d);
    let half = d256.div(&I256::from_i128(env, 2));
    let product = x256.mul(&y256).add(&half);
    to_i128(env, &product.div(&d256))
}

// (x * y) / d (floor).
pub fn mul_div_floor(env: &Env, x: i128, y: i128, d: i128) -> i128 {
    let x256 = I256::from_i128(env, x);
    let y256 = I256::from_i128(env, y);
    let d256 = I256::from_i128(env, d);
    to_i128(env, &x256.mul(&y256).div(&d256))
}

// (x * y) / d (signed half-up).
pub fn mul_div_half_up_signed(env: &Env, x: i128, y: i128, d: i128) -> i128 {
    let x256 = I256::from_i128(env, x);
    let y256 = I256::from_i128(env, y);
    let d256 = I256::from_i128(env, d);
    let half = d256.div(&I256::from_i128(env, 2));
    let product = x256.mul(&y256);
    let zero = I256::from_i128(env, 0);

    let rounded = if product < zero {
        product.sub(&half)
    } else {
        product.add(&half)
    };
    to_i128(env, &rounded.div(&d256))
}

// Rescales precision.
pub fn rescale_half_up(a: i128, from_decimals: u32, to_decimals: u32) -> i128 {
    if from_decimals == to_decimals {
        return a;
    }
    if to_decimals > from_decimals {
        let diff = to_decimals - from_decimals;
        // `checked_pow` over raw `pow` so an over-bound decimal
        // differential produces an explicit panic in both debug and
        // release rather than wrapping silently.
        let factor = 10i128
            .checked_pow(diff)
            .expect("rescale_half_up upscale factor overflow");
        a.checked_mul(factor)
            .expect("rescale_half_up upscale overflow")
    } else {
        let diff = from_decimals - to_decimals;
        let factor = 10i128
            .checked_pow(diff)
            .expect("rescale_half_up downscale factor overflow");
        let half = factor / 2;
        if a >= 0 {
            a.checked_add(half)
                .expect("rescale_half_up rounding overflow")
                / factor
        } else {
            (a - half) / factor
        }
    }
}

// Division with half-up rounding.
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

// I256 to i128 (panics on overflow).
fn to_i128(env: &Env, val: &I256) -> i128 {
    val.to_i128()
        .unwrap_or_else(|| panic_with_error!(env, crate::errors::GenericError::MathOverflow))
}



#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::{RAY, WAD};
    use soroban_sdk::Env;

    #[test]
    fn test_mul_basic() {
        let env = Env::default();
        // 2.0 * 3.0 = 6.0 in RAY
        assert_eq!(mul_div_half_up(&env, 2 * RAY, 3 * RAY, RAY), 6 * RAY);
    }

    #[test]
    fn test_mul_rounding() {
        let env = Env::default();
        // 3 * 0.5 WAD = 1.5, rounds to 2.
        assert_eq!(mul_div_half_up(&env, 3, WAD / 2, WAD), 2);
    }

    #[test]
    fn test_div_basic() {
        let env = Env::default();
        // 6.0 / 3.0 = 2.0 in RAY
        assert_eq!(mul_div_half_up(&env, 6 * RAY, RAY, 3 * RAY), 2 * RAY);
    }

    #[test]
    fn test_div_rounding() {
        let env = Env::default();
        // 1 / 3 in WAD: remainder < 0.5, rounds down.
        assert_eq!(
            mul_div_half_up(&env, WAD, WAD, 3 * WAD),
            333_333_333_333_333_333
        );
        // 2 / 3 in WAD: remainder >= 0.5, rounds up.
        assert_eq!(
            mul_div_half_up(&env, 2 * WAD, WAD, 3 * WAD),
            666_666_666_666_666_667
        );
    }

    #[test]
    fn test_large_values_no_overflow() {
        let env = Env::default();
        // RAY * RAY / RAY = RAY (intermediate is 10^54).
        assert_eq!(mul_div_half_up(&env, RAY, RAY, RAY), RAY);
        assert_eq!(
            mul_div_half_up(&env, 100 * RAY, 100 * RAY, RAY),
            10_000 * RAY
        );
    }

    #[test]
    fn test_signed_positive() {
        let env = Env::default();
        assert_eq!(mul_div_half_up_signed(&env, 3, WAD / 2, WAD), 2);
    }

    #[test]
    fn test_signed_negative() {
        let env = Env::default();
        // -3 * 0.5 = -1.5, rounds away from zero to -2.
        assert_eq!(mul_div_half_up_signed(&env, -3, WAD / 2, WAD), -2);
    }

    #[test]
    fn test_rescale_upscale() {
        // 1.0 at 6 decimals -> 18 decimals.
        assert_eq!(rescale_half_up(1_000_000, 6, 18), 1_000_000_000_000_000_000);
    }

    #[test]
    fn test_rescale_downscale() {
        assert_eq!(rescale_half_up(WAD, 18, 6), 1_000_000);
    }

    #[test]
    fn test_rescale_downscale_rounding() {
        // 0.0000015 at 18 dec -> 6 dec: rounds up from 1.5 to 2.
        assert_eq!(rescale_half_up(1_500_000_000_000, 18, 6), 2);
    }

    #[test]
    fn test_rescale_same() {
        assert_eq!(rescale_half_up(42, 18, 18), 42);
    }

    #[test]
    fn test_rescale_downscale_negative_rounds_away_from_zero() {
        // -0.0000015 at 18 dec -> 6 dec: rounds to -2 (away from zero).
        assert_eq!(rescale_half_up(-1_500_000_000_000, 18, 6), -2);
        // -0.0000001 at 18 dec -> 6 dec: remainder < 0.5, rounds to 0.
        assert_eq!(rescale_half_up(-100_000_000_000, 18, 6), 0);
    }

    #[test]
    #[should_panic(expected = "rescale_half_up upscale overflow")]
    fn test_rescale_upscale_overflow_panics_explicitly() {
        // i128::MAX / 10^27 ~= 1.7e11. 10^20 * 10^27 overflows.
        let huge = 10i128.pow(20);
        rescale_half_up(huge, 0, 27);
    }

    #[test]
    fn test_div_by_int_half_up() {
        assert_eq!(div_by_int_half_up(7, 2), 4); // 3.5 -> 4
        assert_eq!(div_by_int_half_up(6, 4), 2); // 1.5 -> 2
    }

    #[test]
    fn test_div_by_int_half_up_negative_rounds_away_from_zero() {
        assert_eq!(div_by_int_half_up(-7, 2), -4); // -3.5 -> -4
        assert_eq!(div_by_int_half_up(-6, 4), -2); // -1.5 -> -2
        assert_eq!(div_by_int_half_up(-5, 4), -1); // -1.25 -> -1 (remainder < 0.5).
    }

    // -----------------------------------------------------------------
    // Adversarial / edge-case coverage — additions to defend the
    // half-rounding tie-breaker, the overflow boundary, and the
    // negative-dividend behaviour that's documented but not exercised.
    // -----------------------------------------------------------------

    // Exactly 0.5 — the half-up tie-breaker MUST round up for positive
    // results. `1 * 1 + 1 = 2; 2 / 2 = 1`. If the tie-breaker were
    // half-even or half-down this would return 0.
    #[test]
    fn test_mul_div_half_up_exact_half_rounds_up() {
        let env = Env::default();
        assert_eq!(mul_div_half_up(&env, 1, 1, 2), 1);
        // 3/2 = 1.5 → 2; 5/2 = 2.5 → 3; 7/2 = 3.5 → 4.
        assert_eq!(mul_div_half_up(&env, 3, 1, 2), 2);
        assert_eq!(mul_div_half_up(&env, 5, 1, 2), 3);
        assert_eq!(mul_div_half_up(&env, 7, 1, 2), 4);
    }

    // Negative product with `mul_div_half_up`: the function is documented
    // as half-up for positive results. For negatives the `+ d/2` step
    // pulls the result toward zero, so -1.5 rounds to -1 (NOT -2). This
    // is the trap: consumers wanting Banker's-rounding-style symmetric
    // behaviour must use `mul_div_half_up_signed` instead.
    #[test]
    fn test_mul_div_half_up_negative_rounds_toward_zero() {
        let env = Env::default();
        // -1 * 1 + 1 = 0; 0 / 2 = 0. So -0.5 → 0 (toward zero).
        assert_eq!(mul_div_half_up(&env, -1, 1, 2), 0);
        // -3 * 1 + 1 = -2; -2 / 2 = -1. So -1.5 → -1 (toward zero).
        assert_eq!(mul_div_half_up(&env, -3, 1, 2), -1);
    }

    // I256 intermediate is wide enough to hold any `i128 * i128`. Result
    // fits i128 only if `|x * y| / d <= i128::MAX`. With x = y = i128::MAX
    // and d = 1, the result is i128::MAX² which overflows i128 →
    // `to_i128` panics with `MathOverflow`.
    #[test]
    #[should_panic]
    fn test_mul_div_half_up_overflow_panics() {
        let env = Env::default();
        let _ = mul_div_half_up(&env, i128::MAX, i128::MAX, 1);
    }

    // `mul_div_floor` is named "floor" but Rust integer `/` truncates
    // toward zero. For negatives the two semantics diverge: true floor of
    // -7/3 is -3, truncation is -2. Pin the documented behaviour so a
    // future name change to "trunc" doesn't silently flip semantics.
    #[test]
    fn test_mul_div_floor_negative_truncates_toward_zero() {
        let env = Env::default();
        // -7 / 3 → -2 (Rust truncation), NOT -3 (mathematical floor).
        assert_eq!(mul_div_floor(&env, -7, 1, 3), -2);
        // -6 / 3 → -2 exact, no remainder.
        assert_eq!(mul_div_floor(&env, -6, 1, 3), -2);
        // 7 / 3 → 2 (positive — same direction as truncation).
        assert_eq!(mul_div_floor(&env, 7, 1, 3), 2);
    }

    #[test]
    #[should_panic]
    fn test_mul_div_floor_overflow_panics() {
        let env = Env::default();
        let _ = mul_div_floor(&env, i128::MAX, i128::MAX, 1);
    }

    // Signed variant — exactly ±0.5 should round AWAY from zero.
    #[test]
    fn test_mul_div_half_up_signed_exact_half() {
        let env = Env::default();
        // +0.5 → 1 (away from zero, upward).
        assert_eq!(mul_div_half_up_signed(&env, 1, 1, 2), 1);
        // -0.5 → -1 (away from zero, downward).
        assert_eq!(mul_div_half_up_signed(&env, -1, 1, 2), -1);
        // +2.5 → 3, -2.5 → -3.
        assert_eq!(mul_div_half_up_signed(&env, 5, 1, 2), 3);
        assert_eq!(mul_div_half_up_signed(&env, -5, 1, 2), -3);
    }

    // Signed variant — product exactly zero takes the `>=` branch (adds
    // +half), which is mathematically equivalent to no rounding offset
    // here since 0 + half then / d = 0.
    #[test]
    fn test_mul_div_half_up_signed_zero_input() {
        let env = Env::default();
        assert_eq!(mul_div_half_up_signed(&env, 0, 1, 2), 0);
        assert_eq!(mul_div_half_up_signed(&env, 0, 1_000_000, RAY), 0);
    }

    #[test]
    #[should_panic]
    fn test_mul_div_half_up_signed_overflow_panics() {
        let env = Env::default();
        let _ = mul_div_half_up_signed(&env, i128::MAX, i128::MAX, 1);
    }

    // Rescale downscale at exactly half — the rounding tie-breaker.
    // 5 at 1 decimal → 0 decimals: exact = 0.5 → should round to 1.
    #[test]
    fn test_rescale_downscale_exact_half_rounds_up() {
        // (5 + 5) / 10 = 1.
        assert_eq!(rescale_half_up(5, 1, 0), 1);
        // 0.50 with 2 decimals → 0 decimals: (50 + 50) / 100 = 1.
        assert_eq!(rescale_half_up(50, 2, 0), 1);
    }

    // Negative half boundary — `(a - half) / factor` rounds away from
    // zero. -5 at 1 dec → 0 dec: (-5 - 5) / 10 = -1.
    #[test]
    fn test_rescale_downscale_negative_exact_half() {
        assert_eq!(rescale_half_up(-5, 1, 0), -1);
        assert_eq!(rescale_half_up(-50, 2, 0), -1);
    }

    // Identity branch — same decimals returns the input as-is.
    #[test]
    fn test_rescale_same_decimals_returns_input() {
        assert_eq!(rescale_half_up(i128::MAX, 18, 18), i128::MAX);
        assert_eq!(rescale_half_up(i128::MIN, 7, 7), i128::MIN);
        assert_eq!(rescale_half_up(0, 0, 0), 0);
    }

    // Downscale `checked_pow` overflow — `from - to >= 39` exceeds 10^38
    // (i128 cap). Confirms the `expect("downscale factor overflow")`
    // fires rather than silently wrapping.
    #[test]
    #[should_panic(expected = "downscale factor overflow")]
    fn test_rescale_downscale_factor_overflow_panics() {
        // 10^39 doesn't fit i128.
        let _ = rescale_half_up(0, 50, 11);
    }

    // Rounding-overflow inside downscale: `a` near i128::MAX plus the
    // half-step overflows the `checked_add`.
    #[test]
    #[should_panic(expected = "rescale_half_up rounding overflow")]
    fn test_rescale_downscale_rounding_overflow_panics() {
        // factor = 10, half = 5. i128::MAX + 5 overflows.
        let _ = rescale_half_up(i128::MAX, 1, 0);
    }

    // `div_by_int_half_up` overflow on the `a + half_b` step.
    #[test]
    #[should_panic(expected = "div_by_int_half_up rounding overflow")]
    fn test_div_by_int_half_up_addition_overflow_panics() {
        // half_b = 1; i128::MAX + 1 overflows.
        let _ = div_by_int_half_up(i128::MAX, 2);
    }

    // Negative half boundary for `div_by_int_half_up`: `-1 - 1 = -2`,
    // `-2 / 2 = -1`. So -0.5 rounds to -1 (away from zero).
    #[test]
    fn test_div_by_int_half_up_negative_exact_half() {
        assert_eq!(div_by_int_half_up(-1, 2), -1);
        assert_eq!(div_by_int_half_up(-3, 2), -2);
    }
}
