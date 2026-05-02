use soroban_sdk::{panic_with_error, Env, I256};

/// Core fixed-point primitive: computes `(x * y + d/2) / d` using an I256
/// intermediate to prevent overflow. Half-up rounding (0.5 rounds away from
/// zero for positive results).
///
/// Backs every typed operation on [`super::fp::Ray`],
/// [`super::fp::Wad`], and [`super::fp::Bps`].
///
/// # Usage patterns
/// - **Multiply**: `mul_div_half_up(env, a, b, PRECISION)` -> `a * b / precision`
/// - **Divide**:   `mul_div_half_up(env, a, PRECISION, b)` -> `a * precision / b`
pub fn mul_div_half_up(env: &Env, x: i128, y: i128, d: i128) -> i128 {
    let x256 = I256::from_i128(env, x);
    let y256 = I256::from_i128(env, y);
    let d256 = I256::from_i128(env, d);
    let half = d256.div(&I256::from_i128(env, 2));
    let product = x256.mul(&y256).add(&half);
    to_i128(env, &product.div(&d256))
}

/// Floor (truncating-toward-zero) variant: `(x * y) / d` with no rounding bias.
/// Used where the caller needs a guaranteed lower bound (e.g., the base side
/// of the liquidation seizure split, so that the bonus side is never
/// understated and the protocol fee is at least the spec value).
pub fn mul_div_floor(env: &Env, x: i128, y: i128, d: i128) -> i128 {
    let x256 = I256::from_i128(env, x);
    let y256 = I256::from_i128(env, y);
    let d256 = I256::from_i128(env, d);
    to_i128(env, &x256.mul(&y256).div(&d256))
}

/// Signed variant: rounds away from zero for negative results.
/// `-1.5 -> -2`, `1.5 -> 2`.
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

/// Rescales a value between decimal precisions.
/// - Upscale (to > from): checked multiplication. Panics with an explicit
///   "rescale_half_up upscale overflow" message rather than wrap silently.
/// - Downscale (to < from): half-up rounding (away from zero for negatives).
/// - Same: identity.
pub fn rescale_half_up(a: i128, from_decimals: u32, to_decimals: u32) -> i128 {
    if from_decimals == to_decimals {
        return a;
    }
    if to_decimals > from_decimals {
        let diff = to_decimals - from_decimals;
        let factor = 10i128.pow(diff);
        a.checked_mul(factor)
            .expect("rescale_half_up upscale overflow")
    } else {
        let diff = from_decimals - to_decimals;
        let factor = 10i128.pow(diff);
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

/// Computes integer division `(a + sign(a)*b/2) / b` with half-up rounding,
/// rounding away from zero for negatives. Panics if `b == 0`. Requires no `Env`.
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

/// Overflow-safe I256 -> i128 conversion. Panics with `MathOverflow` on failure.
fn to_i128(env: &Env, val: &I256) -> i128 {
    val.to_i128()
        .unwrap_or_else(|| panic_with_error!(env, crate::errors::GenericError::MathOverflow))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
}
