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
fn try_mul_div_half_up_matches_panicking_path_when_in_range() {
    let env = Env::default();
    assert_eq!(
        try_mul_div_half_up(&env, 2 * WAD, 3 * WAD, WAD),
        Some(mul_div_half_up(&env, 2 * WAD, 3 * WAD, WAD))
    );
}

#[test]
fn try_mul_div_half_up_softens_i128_overflow() {
    let env = Env::default();
    // Intermediate fits I256; result does not fit i128.
    assert_eq!(try_mul_div_half_up(&env, i128::MAX, i128::MAX, 1), None);
}

#[test]
fn try_mul_div_half_up_rejects_non_positive_divisor_or_negatives() {
    let env = Env::default();
    assert_eq!(try_mul_div_half_up(&env, 1, 1, 0), None);
    assert_eq!(try_mul_div_half_up(&env, -1, 1, 1), None);
    assert_eq!(try_mul_div_half_up(&env, 1, -1, 1), None);
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

// Positive half-up boundary: 1*1+1=2, 2/2=1.
// Half-even or half-down returns 0.
#[test]
fn test_mul_div_half_up_exact_half_rounds_up() {
    let env = Env::default();
    assert_eq!(mul_div_half_up(&env, 1, 1, 2), 1);
    // 3/2 = 1.5 -> 2; 5/2 = 2.5 -> 3; 7/2 = 3.5 -> 4.
    assert_eq!(mul_div_half_up(&env, 3, 1, 2), 2);
    assert_eq!(mul_div_half_up(&env, 5, 1, 2), 3);
    assert_eq!(mul_div_half_up(&env, 7, 1, 2), 4);
}

// I256 holds any i128*i128, but the result fits i128 only if |x*y|/d <= i128::MAX.
// With x=y=i128::MAX and d=1, `to_i128` panics with `MathOverflow`.
#[test]
#[should_panic]
fn test_mul_div_half_up_overflow_panics() {
    let env = Env::default();
    let _ = mul_div_half_up(&env, i128::MAX, i128::MAX, 1);
}

// `mul_div_floor` uses Rust `/`, which truncates toward zero. For -7/3,
// mathematical floor is -3 but truncation is -2.
#[test]
fn test_mul_div_floor_negative_truncates_toward_zero() {
    let env = Env::default();
    // -7 / 3 = -2 (Rust truncation), not -3 (mathematical floor).
    assert_eq!(mul_div_floor(&env, -7, 1, 3), -2);
    // -6 / 3 = -2 exactly, no remainder.
    assert_eq!(mul_div_floor(&env, -6, 1, 3), -2);
    // 7 / 3 = 2, same direction as truncation.
    assert_eq!(mul_div_floor(&env, 7, 1, 3), 2);
}

#[test]
#[should_panic]
fn test_mul_div_floor_overflow_panics() {
    let env = Env::default();
    let _ = mul_div_floor(&env, i128::MAX, i128::MAX, 1);
}

// `mul_div_ceil` rounds any non-exact non-negative quotient up; exact
// quotients return unchanged.
#[test]
fn test_mul_div_ceil_rounds_up_on_remainder() {
    let env = Env::default();
    // 1/3 -> ceil 1; 1/2 -> ceil 1; exact 6/3 -> 2.
    assert_eq!(mul_div_ceil(&env, 1, 1, 3), 1);
    assert_eq!(mul_div_ceil(&env, 1, 1, 2), 1);
    assert_eq!(mul_div_ceil(&env, 6, 1, 3), 2);
    // WAD-scale: 1/3 in WAD ends in ...334 (one above floor's ...333).
    assert_eq!(
        mul_div_ceil(&env, WAD, WAD, 3 * WAD),
        333_333_333_333_333_334
    );
    assert_eq!(mul_div_ceil(&env, 0, 1, 7), 0);
}

#[test]
#[should_panic]
fn test_mul_div_ceil_overflow_panics() {
    let env = Env::default();
    let _ = mul_div_ceil(&env, i128::MAX, i128::MAX, 1);
}

// Rescale downscale at exact half, the rounding tie-breaker.
// 5 at 1 decimal -> 0 decimals: exact = 0.5 -> rounds to 1.
#[test]
fn test_rescale_downscale_exact_half_rounds_up() {
    // (5 + 5) / 10 = 1.
    assert_eq!(rescale_half_up(5, 1, 0), 1);
    // 0.50 with 2 decimals → 0 decimals: (50 + 50) / 100 = 1.
    assert_eq!(rescale_half_up(50, 2, 0), 1);
}

// Negative half boundary: `(a - half) / factor` rounds away from zero.
// -5 at 1 dec -> 0 dec: (-5 - 5) / 10 = -1.
#[test]
fn test_rescale_downscale_negative_exact_half() {
    assert_eq!(rescale_half_up(-5, 1, 0), -1);
    assert_eq!(rescale_half_up(-50, 2, 0), -1);
}

// Identity branch: same decimals returns the input as-is.
#[test]
fn test_rescale_same_decimals_returns_input() {
    assert_eq!(rescale_half_up(42, 18, 18), 42);
    assert_eq!(rescale_half_up(i128::MAX, 18, 18), i128::MAX);
    assert_eq!(rescale_half_up(i128::MIN, 7, 7), i128::MIN);
    assert_eq!(rescale_half_up(0, 0, 0), 0);
}

// Downscale `checked_pow` overflow: `from - to >= 39` exceeds 10^38 (i128 cap);
// confirms the `expect("downscale factor overflow")` fires, not silent wrap.
#[test]
#[should_panic(expected = "downscale factor overflow")]
fn test_rescale_downscale_factor_overflow_panics() {
    // 10^39 doesn't fit i128.
    let _ = rescale_half_up(0, 50, 11);
}

// Downscale near i128::MAX no longer overflows: the half-up is computed via
// quotient/remainder, not `a + half`, and the result fits (downscale shrinks).
#[test]
fn test_rescale_downscale_near_max_does_not_overflow() {
    // i128::MAX at 1 dec -> 0 dec: floor(MAX/10) with half-up on the last digit.
    let expected = i128::MAX / 10 + if i128::MAX % 10 >= 5 { 1 } else { 0 };
    assert_eq!(rescale_half_up(i128::MAX, 1, 0), expected);
}

// `div_by_int_half_up` overflow on the `a + half_b` step.
#[test]
#[should_panic(expected = "div_by_int_half_up rounding overflow")]
fn test_div_by_int_half_up_addition_overflow_panics() {
    // half_b = 1; i128::MAX + 1 overflows.
    let _ = div_by_int_half_up(i128::MAX, 2);
}

// Negative half boundary for `div_by_int_half_up`: `-1 - 1 = -2`,
// `-2 / 2 = -1`; -0.5 rounds to -1 (away from zero).
#[test]
fn test_div_by_int_half_up_negative_exact_half() {
    assert_eq!(div_by_int_half_up(-1, 2), -1);
    assert_eq!(div_by_int_half_up(-3, 2), -2);
}

// rescale_floor branches.

#[test]
fn test_rescale_floor_identity_returns_input() {
    assert_eq!(rescale_floor(123_456_789, 7, 7), 123_456_789);
    assert_eq!(rescale_floor(i128::MAX, 18, 18), i128::MAX);
    assert_eq!(rescale_floor(0, 27, 27), 0);
}

#[test]
fn test_rescale_floor_upscale_is_exact() {
    // 1 at 6 dec -> 18 dec: 1 * 10^12 = 1_000_000_000_000.
    assert_eq!(rescale_floor(1, 6, 18), 1_000_000_000_000);
    // 7 at 0 dec -> 7 dec: 7 * 10^7 = 70_000_000.
    assert_eq!(rescale_floor(7, 0, 7), 70_000_000);
}

#[test]
fn test_rescale_floor_downscale_truncates_toward_zero() {
    // 19 at 1 dec -> 0 dec: floor(1.9) = 1.
    assert_eq!(rescale_floor(19, 1, 0), 1);
    // 1_999_999 at 6 dec -> 0 dec: floor(1.999_999) = 1.
    assert_eq!(rescale_floor(1_999_999, 6, 0), 1);
}

#[test]
#[should_panic(expected = "rescale_floor upscale factor overflow")]
fn test_rescale_floor_upscale_factor_overflow_panics() {
    // 10^39 overflows i128.
    let _ = rescale_floor(1, 0, 39);
}

#[test]
#[should_panic(expected = "rescale_floor upscale overflow")]
fn test_rescale_floor_upscale_value_overflow_panics() {
    // i128::MAX * 10 overflows.
    let _ = rescale_floor(i128::MAX, 0, 1);
}

// rescale_ceil branches.

#[test]
fn test_rescale_ceil_identity_returns_input() {
    assert_eq!(rescale_ceil(987_654, 5, 5), 987_654);
    assert_eq!(rescale_ceil(0, 0, 0), 0);
}

#[test]
fn test_rescale_ceil_upscale_is_exact() {
    assert_eq!(rescale_ceil(3, 0, 6), 3_000_000);
    assert_eq!(rescale_ceil(1, 6, 18), 1_000_000_000_000);
}

#[test]
fn test_rescale_ceil_downscale_rounds_up_on_remainder() {
    // 11 at 1 dec -> 0 dec: 1.1 -> 2.
    assert_eq!(rescale_ceil(11, 1, 0), 2);
    // Exact remainder = 0: returns the truncated quotient (1).
    assert_eq!(rescale_ceil(10, 1, 0), 1);
    // 1_999_999 at 6 dec -> 0 dec: 1.999999 -> 2.
    assert_eq!(rescale_ceil(1_999_999, 6, 0), 2);
}

#[test]
fn test_rescale_ceil_downscale_negative_truncates_toward_zero() {
    // Negative inputs use the truncated quotient without rounding adjustment.
    // -11 at 1 dec -> 0 dec: -11 / 10 = -1 (toward zero).
    assert_eq!(rescale_ceil(-11, 1, 0), -1);
}

#[test]
#[should_panic(expected = "rescale_ceil upscale factor overflow")]
fn test_rescale_ceil_upscale_factor_overflow_panics() {
    let _ = rescale_ceil(1, 0, 39);
}

#[test]
#[should_panic(expected = "rescale_ceil upscale overflow")]
fn test_rescale_ceil_upscale_value_overflow_panics() {
    let _ = rescale_ceil(i128::MAX, 0, 1);
}

// Downscale with `to_decimals > 0` checks `from - to` subtraction.
// A `+` mutation overflows `10^(from + to)` instead of returning floor/ceil.

#[test]
fn test_rescale_floor_downscale_to_nonzero_decimals() {
    assert_eq!(rescale_floor(1_999_999_999, 9, 6), 1_999_999);
    assert_eq!(rescale_floor(5_000_000_000_000_000_000, 18, 6), 5_000_000);
}

#[test]
fn test_rescale_ceil_downscale_to_nonzero_decimals() {
    assert_eq!(rescale_ceil(1_000_000_001, 9, 6), 1_000_001);
    assert_eq!(rescale_ceil(1_000_000_000, 9, 6), 1_000_000);
}

// ---------------------------------------------------------------------------
// geometric_mean_floor: floor(sqrt(a*b)) via I256, AM-GM-seeded Newton.
// ---------------------------------------------------------------------------

#[test]
fn test_geometric_mean_exact_squares() {
    let env = Env::default();
    assert_eq!(geometric_mean_floor(&env, 4, 9), 6);
    assert_eq!(geometric_mean_floor(&env, 100, 100), 100);
    assert_eq!(geometric_mean_floor(&env, 1, 1), 1);
}

#[test]
fn test_geometric_mean_floors_not_rounds() {
    let env = Env::default();
    // sqrt(2*3) = sqrt(6) = 2.449... -> 2, not 3.
    assert_eq!(geometric_mean_floor(&env, 2, 3), 2);
    // sqrt(3*3) = 3 exactly.
    assert_eq!(geometric_mean_floor(&env, 3, 3), 3);
    // sqrt(8*9) = sqrt(72) = 8.485... -> 8.
    assert_eq!(geometric_mean_floor(&env, 8, 9), 8);
}

#[test]
fn test_geometric_mean_zero_operand() {
    let env = Env::default();
    assert_eq!(geometric_mean_floor(&env, 0, 12345), 0);
    assert_eq!(geometric_mean_floor(&env, 12345, 0), 0);
    assert_eq!(geometric_mean_floor(&env, 0, 0), 0);
}

#[test]
fn test_geometric_mean_wad_scale_balanced_pool() {
    let env = Env::default();
    // A balanced pool leg pair: $1000 each side, in WAD. The geometric mean of
    // two equal values is that value, so the LP total is 2x it.
    let leg = 1_000 * WAD;
    assert_eq!(geometric_mean_floor(&env, leg, leg), leg);
}

#[test]
fn test_geometric_mean_overflows_i128_product() {
    let env = Env::default();
    // 1e30 * 1e30 = 1e60, far past i128::MAX (~1.7e38). Only the I256
    // intermediate makes this representable; the root narrows back to 1e30.
    let big = 10i128.pow(30);
    assert_eq!(geometric_mean_floor(&env, big, big), big);
}

#[test]
fn test_geometric_mean_extreme_imbalance_converges() {
    let env = Env::default();
    // 1e12x imbalance: the AM seed sits ~500000x above the answer, which is the
    // worst case for the iteration cap.
    let a = 10i128.pow(6);
    let b = 10i128.pow(30);
    // sqrt(1e36) = 1e18 exactly.
    assert_eq!(geometric_mean_floor(&env, a, b), 10i128.pow(18));
}

#[test]
fn test_geometric_mean_is_symmetric() {
    let env = Env::default();
    for (a, b) in [
        (7i128, 1_000_000i128),
        (3 * WAD, 5 * WAD),
        (1, i128::MAX / 2),
    ] {
        assert_eq!(
            geometric_mean_floor(&env, a, b),
            geometric_mean_floor(&env, b, a),
            "geometric mean must not depend on operand order"
        );
    }
}

#[test]
fn test_geometric_mean_result_squared_brackets_product() {
    let env = Env::default();
    // Defining property of a floor square root: r^2 <= a*b < (r+1)^2. Operands
    // are kept small enough that the product and (r+1)^2 stay inside i128.
    for (a, b) in [
        (6i128, 7i128),
        (99, 101),
        (123_456, 654_321),
        (WAD, 3 * WAD),
    ] {
        let r = geometric_mean_floor(&env, a, b);
        let product = a * b;
        assert!(r * r <= product, "r^2 > a*b for ({a}, {b})");
        assert!((r + 1) * (r + 1) > product, "(r+1)^2 <= a*b for ({a}, {b})");
    }
}

#[test]
#[should_panic]
fn test_geometric_mean_rejects_negative() {
    let env = Env::default();
    let _ = geometric_mean_floor(&env, -1, 4);
}

#[test]
fn test_geometric_mean_at_the_i128_ceiling() {
    // Worst case for both seeds and for the I256 intermediate: MAX^2 is ~2.9e76
    // against an I256 ceiling of ~5.7e76, and the root is MAX itself, which only
    // just fits back into i128.
    let env = Env::default();
    assert_eq!(
        geometric_mean_floor(&env, i128::MAX, i128::MAX),
        i128::MAX,
        "sqrt(MAX^2) must round-trip exactly"
    );
}

#[test]
fn test_geometric_mean_at_adjacent_powers_of_two() {
    // Exercises the bit-length seed where the two operands straddle a power of
    // two, which is where an off-by-one in the exponent would show up as a seed
    // below the true root - and Newton from below does not converge downward.
    let env = Env::default();
    let high = 1i128 << 126;
    assert_eq!(geometric_mean_floor(&env, high, high), high);
    // sqrt(2^126 * (2^126 - 1)) floors to 2^126 - 1.
    assert_eq!(geometric_mean_floor(&env, high, high - 1), high - 1);
}

#[test]
fn test_geometric_mean_extreme_ratio_stays_well_inside_the_iteration_cap() {
    // A brute-force model over 60k structured and random pairs peaks at 8
    // Newton steps; this pins the shape that produces the worst count so a
    // regression in the seed shows up as a revert rather than silent slowness.
    let env = Env::default();
    assert_eq!(
        geometric_mean_floor(&env, 1, i128::MAX),
        13_043_817_825_332_782_212
    );
}
