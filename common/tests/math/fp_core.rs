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
    let env = Env::default();
    assert_eq!(
        rescale_half_up(&env, 1_000_000, 6, 18),
        1_000_000_000_000_000_000
    );
}

#[test]
fn test_rescale_downscale() {
    let env = Env::default();
    assert_eq!(rescale_half_up(&env, WAD, 18, 6), 1_000_000);
}

#[test]
fn test_rescale_downscale_rounding() {
    let env = Env::default();
    assert_eq!(rescale_half_up(&env, 1_500_000_000_000, 18, 6), 2);
}

#[test]
fn test_rescale_downscale_negative_rounds_away_from_zero() {
    let env = Env::default();
    assert_eq!(rescale_half_up(&env, -1_500_000_000_000, 18, 6), -2);

    assert_eq!(rescale_half_up(&env, -100_000_000_000, 18, 6), 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #33)")]
fn test_rescale_upscale_overflow_panics_explicitly() {
    let env = Env::default();
    let huge = 10i128.pow(20);
    rescale_half_up(&env, huge, 0, 27);
}

#[test]
fn test_div_by_int_half_up() {
    let env = Env::default();
    assert_eq!(div_by_int_half_up(&env, 7, 2), 4);
    assert_eq!(div_by_int_half_up(&env, 6, 4), 2);
}

#[test]
fn test_div_by_int_half_up_negative_rounds_away_from_zero() {
    let env = Env::default();
    assert_eq!(div_by_int_half_up(&env, -7, 2), -4);
    assert_eq!(div_by_int_half_up(&env, -6, 4), -2);
    assert_eq!(div_by_int_half_up(&env, -5, 4), -1);
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
fn test_mul_div_floor_rounds_toward_negative_infinity() {
    let env = Env::default();

    // -7/3 = -2.33 -> floor is -3, not the -2 that truncation would give.
    assert_eq!(mul_div_floor(&env, -7, 1, 3), -3);

    // Exact quotients are unaffected by the rounding direction.
    assert_eq!(mul_div_floor(&env, -6, 1, 3), -2);

    assert_eq!(mul_div_floor(&env, 7, 1, 3), 2);

    // A negative divisor puts the quotient on the same side.
    assert_eq!(mul_div_floor(&env, 7, 1, -3), -3);
    assert_eq!(mul_div_floor(&env, -7, 1, -3), 2);
}

#[test]
fn test_mul_div_ceil_rounds_toward_positive_infinity() {
    let env = Env::default();

    assert_eq!(mul_div_ceil(&env, 7, 1, 3), 3);

    // -7/3 = -2.33 -> ceil is -2; the pre-fix body returned -1 here.
    assert_eq!(mul_div_ceil(&env, -7, 1, 3), -2);

    assert_eq!(mul_div_ceil(&env, -6, 1, 3), -2);

    assert_eq!(mul_div_ceil(&env, 7, 1, -3), -2);
    assert_eq!(mul_div_ceil(&env, -7, 1, -3), 3);
}

#[test]
#[should_panic(expected = "Error(Contract, #55)")]
fn test_mul_div_floor_zero_divisor_is_typed_division_by_zero() {
    let env = Env::default();
    let _ = mul_div_floor(&env, 1, 1, 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #55)")]
fn test_mul_div_ceil_zero_divisor_is_typed_division_by_zero() {
    let env = Env::default();
    let _ = mul_div_ceil(&env, 1, 1, 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #55)")]
fn test_mul_div_floor_saturating_zero_divisor_is_typed_division_by_zero() {
    let env = Env::default();
    let _ = mul_div_floor_saturating(&env, 1, 1, 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #55)")]
fn test_div_by_int_half_up_zero_divisor_is_typed_division_by_zero() {
    let env = Env::default();
    let _ = div_by_int_half_up(&env, 1, 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #33)")]
fn test_div_by_int_half_up_negative_divisor_is_typed_math_overflow() {
    let env = Env::default();
    let _ = div_by_int_half_up(&env, i128::MIN, -1);
}

#[test]
fn test_mul_div_floor_saturating_saturates_toward_the_sign_of_the_quotient() {
    let env = Env::default();
    assert_eq!(
        mul_div_floor_saturating(&env, i128::MAX, i128::MAX, 1),
        i128::MAX
    );
    assert_eq!(
        mul_div_floor_saturating(&env, i128::MAX, i128::MAX, -1),
        i128::MIN
    );
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
    let env = Env::default();
    assert_eq!(rescale_half_up(&env, 5, 1, 0), 1);

    assert_eq!(rescale_half_up(&env, 50, 2, 0), 1);
}

#[test]
fn test_rescale_downscale_negative_exact_half() {
    let env = Env::default();
    assert_eq!(rescale_half_up(&env, -5, 1, 0), -1);
    assert_eq!(rescale_half_up(&env, -50, 2, 0), -1);
}

#[test]
fn test_rescale_same_decimals_returns_input() {
    let env = Env::default();
    assert_eq!(rescale_half_up(&env, 42, 18, 18), 42);
    assert_eq!(rescale_half_up(&env, i128::MAX, 18, 18), i128::MAX);
    assert_eq!(rescale_half_up(&env, i128::MIN, 7, 7), i128::MIN);
    assert_eq!(rescale_half_up(&env, 0, 0, 0), 0);
}

#[test]
fn test_rescale_downscale_factor_larger_than_i128_saturates_to_zero() {
    let env = Env::default();
    // 10^39 does not fit i128, so it exceeds every representable `a`: the
    // quotient is 0 for floor and half-up, and 1 for ceil on a positive `a`.
    assert_eq!(rescale_half_up(&env, 0, 50, 11), 0);
    assert_eq!(rescale_half_up(&env, i128::MAX, 50, 11), 0);
    assert_eq!(rescale_half_up(&env, i128::MIN, 50, 11), 0);
    assert_eq!(rescale_floor(&env, i128::MAX, 50, 11), 0);
    assert_eq!(rescale_ceil(&env, i128::MAX, 50, 11), 1);
    assert_eq!(rescale_ceil(&env, 0, 50, 11), 0);
    assert_eq!(rescale_ceil(&env, i128::MIN, 50, 11), 0);
}

#[test]
fn test_rescale_downscale_extremes_do_not_overflow() {
    let env = Env::default();
    assert_eq!(rescale_half_up(&env, i128::MAX, 1, 0), i128::MAX / 10 + 1);
    assert_eq!(rescale_half_up(&env, i128::MIN, 1, 0), i128::MIN / 10 - 1);
    assert_eq!(rescale_half_up(&env, i128::MAX, 38, 0), 2);
    assert_eq!(rescale_half_up(&env, i128::MIN, 38, 0), -2);
}

#[test]
fn test_div_by_int_half_up_extremes_do_not_overflow() {
    let env = Env::default();
    for (a, b, expected) in [
        (i128::MAX, 1, i128::MAX),
        (i128::MIN, 1, i128::MIN),
        (i128::MAX, 2, i128::MAX / 2 + 1),
        (i128::MIN, 2, i128::MIN / 2),
        (i128::MIN + 1, 2, i128::MIN / 2),
        (i128::MAX, 3, i128::MAX / 3),
        (i128::MIN, 3, i128::MIN / 3 - 1),
        (i128::MAX, i128::MAX, 1),
        (i128::MIN, i128::MAX, -1),
        (i128::MAX / 2, i128::MAX, 0),
        (i128::MAX / 2 + 1, i128::MAX, 1),
        (-(i128::MAX / 2), i128::MAX, 0),
        (-(i128::MAX / 2) - 1, i128::MAX, -1),
    ] {
        assert_eq!(div_by_int_half_up(&env, a, b), expected, "{a} / {b}");
    }
}

#[test]
fn test_div_by_int_half_up_signed_rounding_boundaries() {
    let env = Env::default();
    for (a, b, expected) in [
        (0, 1, 0),
        (0, 3, 0),
        (1, 3, 0),
        (2, 3, 1),
        (3, 3, 1),
        (4, 3, 1),
        (5, 3, 2),
        (1, 4, 0),
        (2, 4, 1),
        (3, 4, 1),
        (5, 4, 1),
        (6, 4, 2),
        (7, 4, 2),
    ] {
        assert_eq!(div_by_int_half_up(&env, a, b), expected, "{a} / {b}");
        assert_eq!(div_by_int_half_up(&env, -a, b), -expected, "-({a}) / {b}");
    }
}

#[test]
fn test_div_by_int_half_up_negative_exact_half() {
    let env = Env::default();
    assert_eq!(div_by_int_half_up(&env, -1, 2), -1);
    assert_eq!(div_by_int_half_up(&env, -3, 2), -2);
}

#[test]
fn test_rescale_floor_identity_returns_input() {
    let env = Env::default();
    assert_eq!(rescale_floor(&env, 123_456_789, 7, 7), 123_456_789);
    assert_eq!(rescale_floor(&env, i128::MAX, 18, 18), i128::MAX);
    assert_eq!(rescale_floor(&env, 0, 27, 27), 0);
}

#[test]
fn test_rescale_floor_upscale_is_exact() {
    let env = Env::default();
    assert_eq!(rescale_floor(&env, 1, 6, 18), 1_000_000_000_000);

    assert_eq!(rescale_floor(&env, 7, 0, 7), 70_000_000);
}

#[test]
fn test_rescale_floor_downscale_truncates_toward_zero() {
    let env = Env::default();
    assert_eq!(rescale_floor(&env, 19, 1, 0), 1);

    assert_eq!(rescale_floor(&env, 1_999_999, 6, 0), 1);
}

#[test]
#[should_panic(expected = "Error(Contract, #33)")]
fn test_rescale_floor_upscale_factor_overflow_panics() {
    let env = Env::default();
    let _ = rescale_floor(&env, 1, 0, 39);
}

#[test]
#[should_panic(expected = "Error(Contract, #33)")]
fn test_rescale_floor_upscale_value_overflow_panics() {
    let env = Env::default();
    let _ = rescale_floor(&env, i128::MAX, 0, 1);
}

#[test]
fn test_rescale_ceil_identity_returns_input() {
    let env = Env::default();
    assert_eq!(rescale_ceil(&env, 987_654, 5, 5), 987_654);
    assert_eq!(rescale_ceil(&env, 0, 0, 0), 0);
}

#[test]
fn test_rescale_ceil_upscale_is_exact() {
    let env = Env::default();
    assert_eq!(rescale_ceil(&env, 3, 0, 6), 3_000_000);
    assert_eq!(rescale_ceil(&env, 1, 6, 18), 1_000_000_000_000);
}

#[test]
fn test_rescale_ceil_downscale_rounds_up_on_remainder() {
    let env = Env::default();
    assert_eq!(rescale_ceil(&env, 11, 1, 0), 2);

    assert_eq!(rescale_ceil(&env, 10, 1, 0), 1);

    assert_eq!(rescale_ceil(&env, 1_999_999, 6, 0), 2);
}

#[test]
fn test_rescale_ceil_downscale_negative_truncates_toward_zero() {
    let env = Env::default();
    assert_eq!(rescale_ceil(&env, -11, 1, 0), -1);
}

#[test]
#[should_panic(expected = "Error(Contract, #33)")]
fn test_rescale_ceil_upscale_factor_overflow_panics() {
    let env = Env::default();
    let _ = rescale_ceil(&env, 1, 0, 39);
}

#[test]
#[should_panic(expected = "Error(Contract, #33)")]
fn test_rescale_ceil_upscale_value_overflow_panics() {
    let env = Env::default();
    let _ = rescale_ceil(&env, i128::MAX, 0, 1);
}

#[test]
fn test_rescale_floor_downscale_to_nonzero_decimals() {
    let env = Env::default();
    assert_eq!(rescale_floor(&env, 1_999_999_999, 9, 6), 1_999_999);
    assert_eq!(
        rescale_floor(&env, 5_000_000_000_000_000_000, 18, 6),
        5_000_000
    );
}

#[test]
fn test_rescale_ceil_downscale_to_nonzero_decimals() {
    let env = Env::default();
    assert_eq!(rescale_ceil(&env, 1_000_000_001, 9, 6), 1_000_001);
    assert_eq!(rescale_ceil(&env, 1_000_000_000, 9, 6), 1_000_000);
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
    let env = Env::default();
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
                rescale_half_up(&env, a, from, to),
                ref_half_up(a, from, to),
                "half_up {a} {from}->{to}"
            );
            assert_eq!(
                rescale_floor(&env, a, from, to),
                ref_floor(a, from, to),
                "floor {a} {from}->{to}"
            );
            assert_eq!(
                rescale_ceil(&env, a, from, to),
                ref_ceil(a, from, to),
                "ceil {a} {from}->{to}"
            );
        }
    }
}
