use super::*;
use crate::constants::{RAY, WAD};
use soroban_sdk::Env;

#[test]
fn test_mul_basic() {
    let env = Env::default();

    assert_eq!(mul_div_half_up(&env, 2 * RAY, 3 * RAY, RAY), 6 * RAY);
}

#[test]
fn test_mul_rounding() {
    let env = Env::default();

    assert_eq!(mul_div_half_up(&env, 3, WAD / 2, WAD), 2);
}

#[test]
fn test_div_basic() {
    let env = Env::default();

    assert_eq!(mul_div_half_up(&env, 6 * RAY, RAY, 3 * RAY), 2 * RAY);
}

#[test]
fn test_div_rounding() {
    let env = Env::default();

    assert_eq!(
        mul_div_half_up(&env, WAD, WAD, 3 * WAD),
        333_333_333_333_333_333
    );

    assert_eq!(
        mul_div_half_up(&env, 2 * WAD, WAD, 3 * WAD),
        666_666_666_666_666_667
    );
}

#[test]
fn test_large_values_no_overflow() {
    let env = Env::default();

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
    assert_eq!(rescale_half_up(1_000_000, 6, 18), 1_000_000_000_000_000_000);
}

#[test]
fn test_rescale_downscale() {
    assert_eq!(rescale_half_up(WAD, 18, 6), 1_000_000);
}

#[test]
fn test_rescale_downscale_rounding() {
    assert_eq!(rescale_half_up(1_500_000_000_000, 18, 6), 2);
}

#[test]
fn test_rescale_downscale_negative_rounds_away_from_zero() {
    assert_eq!(rescale_half_up(-1_500_000_000_000, 18, 6), -2);

    assert_eq!(rescale_half_up(-100_000_000_000, 18, 6), 0);
}

#[test]
#[should_panic(expected = "rescale upscale overflow")]
fn test_rescale_upscale_overflow_panics_explicitly() {
    let huge = 10i128.pow(20);
    rescale_half_up(huge, 0, 27);
}

#[test]
fn test_div_by_int_half_up() {
    assert_eq!(div_by_int_half_up(7, 2), 4);
    assert_eq!(div_by_int_half_up(6, 4), 2);
}

#[test]
fn test_div_by_int_half_up_negative_rounds_away_from_zero() {
    assert_eq!(div_by_int_half_up(-7, 2), -4);
    assert_eq!(div_by_int_half_up(-6, 4), -2);
    assert_eq!(div_by_int_half_up(-5, 4), -1);
}

#[test]
fn test_mul_div_half_up_exact_half_rounds_up() {
    let env = Env::default();
    assert_eq!(mul_div_half_up(&env, 1, 1, 2), 1);

    assert_eq!(mul_div_half_up(&env, 3, 1, 2), 2);
    assert_eq!(mul_div_half_up(&env, 5, 1, 2), 3);
    assert_eq!(mul_div_half_up(&env, 7, 1, 2), 4);
}

#[test]
#[should_panic]
fn test_mul_div_half_up_overflow_panics() {
    let env = Env::default();
    let _ = mul_div_half_up(&env, i128::MAX, i128::MAX, 1);
}

#[test]
fn test_mul_div_floor_negative_truncates_toward_zero() {
    let env = Env::default();

    assert_eq!(mul_div_floor(&env, -7, 1, 3), -2);

    assert_eq!(mul_div_floor(&env, -6, 1, 3), -2);

    assert_eq!(mul_div_floor(&env, 7, 1, 3), 2);
}

#[test]
#[should_panic]
fn test_mul_div_floor_overflow_panics() {
    let env = Env::default();
    let _ = mul_div_floor(&env, i128::MAX, i128::MAX, 1);
}

#[test]
fn test_mul_div_ceil_rounds_up_on_remainder() {
    let env = Env::default();

    assert_eq!(mul_div_ceil(&env, 1, 1, 3), 1);
    assert_eq!(mul_div_ceil(&env, 1, 1, 2), 1);
    assert_eq!(mul_div_ceil(&env, 6, 1, 3), 2);

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

#[test]
fn test_rescale_downscale_exact_half_rounds_up() {
    assert_eq!(rescale_half_up(5, 1, 0), 1);

    assert_eq!(rescale_half_up(50, 2, 0), 1);
}

#[test]
fn test_rescale_downscale_negative_exact_half() {
    assert_eq!(rescale_half_up(-5, 1, 0), -1);
    assert_eq!(rescale_half_up(-50, 2, 0), -1);
}

#[test]
fn test_rescale_same_decimals_returns_input() {
    assert_eq!(rescale_half_up(42, 18, 18), 42);
    assert_eq!(rescale_half_up(i128::MAX, 18, 18), i128::MAX);
    assert_eq!(rescale_half_up(i128::MIN, 7, 7), i128::MIN);
    assert_eq!(rescale_half_up(0, 0, 0), 0);
}

#[test]
#[should_panic(expected = "rescale factor overflow")]
fn test_rescale_downscale_factor_overflow_panics() {
    let _ = rescale_half_up(0, 50, 11);
}

#[test]
fn test_rescale_downscale_near_max_does_not_overflow() {
    let expected = i128::MAX / 10 + if i128::MAX % 10 >= 5 { 1 } else { 0 };
    assert_eq!(rescale_half_up(i128::MAX, 1, 0), expected);
}

#[test]
#[should_panic(expected = "div_by_int_half_up rounding overflow")]
fn test_div_by_int_half_up_addition_overflow_panics() {
    let _ = div_by_int_half_up(i128::MAX, 2);
}

#[test]
fn test_div_by_int_half_up_negative_exact_half() {
    assert_eq!(div_by_int_half_up(-1, 2), -1);
    assert_eq!(div_by_int_half_up(-3, 2), -2);
}

#[test]
fn test_rescale_floor_identity_returns_input() {
    assert_eq!(rescale_floor(123_456_789, 7, 7), 123_456_789);
    assert_eq!(rescale_floor(i128::MAX, 18, 18), i128::MAX);
    assert_eq!(rescale_floor(0, 27, 27), 0);
}

#[test]
fn test_rescale_floor_upscale_is_exact() {
    assert_eq!(rescale_floor(1, 6, 18), 1_000_000_000_000);

    assert_eq!(rescale_floor(7, 0, 7), 70_000_000);
}

#[test]
fn test_rescale_floor_downscale_truncates_toward_zero() {
    assert_eq!(rescale_floor(19, 1, 0), 1);

    assert_eq!(rescale_floor(1_999_999, 6, 0), 1);
}

#[test]
#[should_panic(expected = "rescale factor overflow")]
fn test_rescale_floor_upscale_factor_overflow_panics() {
    let _ = rescale_floor(1, 0, 39);
}

#[test]
#[should_panic(expected = "rescale upscale overflow")]
fn test_rescale_floor_upscale_value_overflow_panics() {
    let _ = rescale_floor(i128::MAX, 0, 1);
}

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
    assert_eq!(rescale_ceil(11, 1, 0), 2);

    assert_eq!(rescale_ceil(10, 1, 0), 1);

    assert_eq!(rescale_ceil(1_999_999, 6, 0), 2);
}

#[test]
fn test_rescale_ceil_downscale_negative_truncates_toward_zero() {
    assert_eq!(rescale_ceil(-11, 1, 0), -1);
}

#[test]
#[should_panic(expected = "rescale factor overflow")]
fn test_rescale_ceil_upscale_factor_overflow_panics() {
    let _ = rescale_ceil(1, 0, 39);
}

#[test]
#[should_panic(expected = "rescale upscale overflow")]
fn test_rescale_ceil_upscale_value_overflow_panics() {
    let _ = rescale_ceil(i128::MAX, 0, 1);
}

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

/// `mul_div_floor_saturating` is the only mul_div variant that does NOT panic on
/// overflow — it returns `i128::MAX`. Every panicking sibling has an overflow
/// test; this one had none, in either the unit tests or the fuzzer, despite
/// being the variant used for interest index growth
/// (`common/src/rates/index.rs:41`) and fee-to-share conversion (`:95`).
///
/// The contrast is the point: identical inputs, one aborts and one silently
/// saturates. A silent `i128::MAX` in index growth would be a catastrophic
/// state rather than a rejected transaction, so the behaviour deserves to be
/// pinned rather than left implicit.
#[test]
fn mul_div_floor_saturating_saturates_where_the_panicking_variant_aborts() {
    let env = Env::default();
    // mul_div_floor panics on exactly these inputs (see
    // test_mul_div_floor_overflow_panics); the saturating variant must not.
    assert_eq!(
        mul_div_floor_saturating(&env, i128::MAX, i128::MAX, 1),
        i128::MAX
    );
}

/// Below the saturation point it must agree exactly with the panicking floor
/// variant, or the two would disagree on ordinary values and the choice of
/// variant would silently change results.
#[test]
fn mul_div_floor_saturating_matches_floor_below_saturation() {
    let env = Env::default();
    for (x, y, d) in [
        (0i128, 5i128, 3i128),
        (1, 1, 1),
        (7, 3, 2),
        (WAD, WAD, WAD),
        (RAY, 3, 7),
        (i128::MAX, 1, 1),
        // Exact division: no rounding either way.
        (100, 10, 5),
        // Inexact: floor must truncate, not round.
        (99, 10, 7),
    ] {
        assert_eq!(
            mul_div_floor_saturating(&env, x, y, d),
            mul_div_floor(&env, x, y, d),
            "saturating and panicking floor disagree at ({x}, {y}, {d})"
        );
    }
}

/// Pins the collapsed [`rescale`] against the three hand-written bodies it replaced, across
/// negative, zero, exact-multiple and half-boundary inputs in both directions.
#[test]
fn test_rescale_variants_match_their_pre_refactor_bodies() {
    fn ref_half_up(a: i128, from: u32, to: u32) -> i128 {
        if from == to {
            return a;
        }
        if to > from {
            return a * 10i128.pow(to - from);
        }
        let factor = 10i128.pow(from - to);
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
    fn ref_floor(a: i128, from: u32, to: u32) -> i128 {
        if from == to {
            return a;
        }
        if to > from {
            return a * 10i128.pow(to - from);
        }
        a / 10i128.pow(from - to)
    }
    fn ref_ceil(a: i128, from: u32, to: u32) -> i128 {
        if from == to {
            return a;
        }
        if to > from {
            return a * 10i128.pow(to - from);
        }
        let factor = 10i128.pow(from - to);
        let quotient = a / factor;
        let remainder = a % factor;
        if a >= 0 && remainder != 0 {
            quotient + 1
        } else {
            quotient
        }
    }

    for &(from, to) in &[
        (7u32, 7u32),
        (6, 18),
        (0, 7),
        (18, 7),
        (18, 0),
        (27, 18),
        (1, 0),
    ] {
        for a in [
            0i128,
            1,
            -1,
            5,
            -5,
            50,
            -50,
            499_999,
            500_000,
            500_001,
            -499_999,
            -500_000,
            -500_001,
            1_000_000,
            -1_000_000,
            1_999_999,
            -1_999_999,
            123_456_789,
            -123_456_789,
        ] {
            assert_eq!(
                rescale_half_up(a, from, to),
                ref_half_up(a, from, to),
                "half_up {a} {from}->{to}"
            );
            assert_eq!(
                rescale_floor(a, from, to),
                ref_floor(a, from, to),
                "floor {a} {from}->{to}"
            );
            assert_eq!(
                rescale_ceil(a, from, to),
                ref_ceil(a, from, to),
                "ceil {a} {from}->{to}"
            );
        }
    }
}
