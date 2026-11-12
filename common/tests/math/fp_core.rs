use super::*;
use crate::constants::{RAY, WAD};
use crate::math::test_support::{
    ceil_fits, ceil_of, contract_error_code, edges, floor_fits, floor_of, half_up_fits, half_up_of,
    is_ceil, is_floor, is_half_away, is_half_up, is_rescaled, quotient_negative, rescale_fits,
    Rounding, Sampler, Wide,
};
use soroban_sdk::{Env, I256};

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

    // A negative divisor also floors.
    assert_eq!(mul_div_floor(&env, 7, 1, -3), -3);
    assert_eq!(mul_div_floor(&env, -7, 1, -3), 2);
}

#[test]
fn test_positive_widened_floor_avoids_remainder_host_call() {
    use soroban_sdk::testutils::budget::ContractCostType;

    let env = Env::default();
    let mut budget = env.cost_estimate().budget();
    assert!(
        RAY.checked_mul(RAY).is_none(),
        "fixture must widen the product"
    );
    budget.reset_tracker();

    assert_eq!(mul_div_floor(&env, RAY, RAY, RAY), RAY);
    assert!(
        budget.tracker(ContractCostType::Int256Div).iterations <= 1,
        "nonnegative floor should not compute a remainder"
    );
}

#[test]
fn test_mul_div_ceil_rounds_toward_positive_infinity() {
    let env = Env::default();

    assert_eq!(mul_div_ceil(&env, 7, 1, 3), 3);

    // -7/3 = -2.33 -> ceil is -2.
    assert_eq!(mul_div_ceil(&env, -7, 1, 3), -2);

    assert_eq!(mul_div_ceil(&env, -6, 1, 3), -2);

    assert_eq!(mul_div_ceil(&env, 7, 1, -3), -2);
    assert_eq!(mul_div_ceil(&env, -7, 1, -3), 3);
}

#[test]
fn test_signed_widened_mul_div_rounding_matches_literal_fixtures() {
    let env = Env::default();
    for (x, y, d, floor, ceil) in [
        (
            -100_000_000_000_000_000_000i128,
            100_000_000_000_000_000_000,
            100_000_000_000_000_000_000,
            -100_000_000_000_000_000_000,
            -100_000_000_000_000_000_000,
        ),
        (
            -100_000_000_000_000_000_000,
            100_000_000_000_000_000_000,
            300_000_000_000_000_000_000,
            -33_333_333_333_333_333_334,
            -33_333_333_333_333_333_333,
        ),
        (
            -100_000_000_000_000_000_000,
            -100_000_000_000_000_000_000,
            -300_000_000_000_000_000_000,
            -33_333_333_333_333_333_334,
            -33_333_333_333_333_333_333,
        ),
    ] {
        assert!(x.checked_mul(y).is_none(), "fixture must widen the product");
        assert_eq!(mul_div_floor(&env, x, y, d), floor, "floor {x} {y} {d}");
        assert_eq!(mul_div_ceil(&env, x, y, d), ceil, "ceil {x} {y} {d}");
        assert_eq!(
            mul_div_floor_saturating(&env, x, y, d),
            floor,
            "saturating floor {x} {y} {d}"
        );
    }
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

/// `mul_div_floor_saturating` returns `i128::MAX` on a positive overflow, where `mul_div_floor`
/// panics. `update_supply_index` and `protocol_fee_shares` (`common/src/rates/index.rs`)
/// use it.
#[test]
fn mul_div_floor_saturating_saturates_where_the_panicking_variant_aborts() {
    let env = Env::default();
    // Same inputs as `test_mul_div_floor_overflow_panics`.
    assert_eq!(
        mul_div_floor_saturating(&env, i128::MAX, i128::MAX, 1),
        i128::MAX
    );
}

/// Below saturation, `mul_div_floor_saturating` equals `mul_div_floor`.
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

/// Checks `rescale_half_up`, `rescale_floor` and `rescale_ceil` against reference bodies
/// across negative, zero, exact-multiple and half-boundary inputs in both directions.
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

#[test]
fn test_small_mul_div_floor_and_ceil_need_no_host_division() {
    use soroban_sdk::testutils::budget::ContractCostType;

    let env = Env::default();
    let mut budget = env.cost_estimate().budget();
    budget.reset_tracker();

    assert_eq!(mul_div_floor(&env, 7, 1, 3), 2);
    assert_eq!(mul_div_ceil(&env, 7, 1, 3), 3);
    let divisions = budget.tracker(ContractCostType::Int256Div).iterations;
    assert_eq!(divisions, 0);
}

// ---------------------------------------------------------------------------
// Exact differential tests.
//
// Each test compares a primitive with the exact rational result on every
// combination of the edge values and on seeded random operands of every
// magnitude and sign. `test_support` states the expected rounding through
// inequalities on exact 256-bit products, so a result passes only if it is the
// one integer the rounding rule allows.
// ---------------------------------------------------------------------------

/// Seeded random operand tuples checked by each exact test.
const RANDOM_CASES: usize = 20_000;

const MATH_OVERFLOW: Option<u32> = Some(GenericError::MathOverflow as u32);

fn unmetered_env() -> Env {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    env
}

#[test]
fn exact_native_division_helpers_match_floor_and_ceiling() {
    let check = |p: i128, d: i128| {
        let (floor, ceil) = (div_floor_i128(p, d), div_ceil_i128(p, d));
        if d == 0 || (p == i128::MIN && d == -1) {
            assert_eq!((floor, ceil), (None, None), "{p} / {d}");
            return;
        }
        let n = Wide::from(p);
        assert!(
            floor.is_some_and(|q| is_floor(q, n, d)),
            "floor {p} / {d} = {floor:?}"
        );
        assert!(
            ceil.is_some_and(|q| is_ceil(q, n, d)),
            "ceil {p} / {d} = {ceil:?}"
        );
    };

    for p in edges() {
        for d in edges().chain([-1]) {
            check(p, d);
        }
    }
    let mut sampler = Sampler::new(1);
    for _ in 0..RANDOM_CASES {
        check(sampler.signed(), sampler.signed());
    }
}

#[test]
fn exact_quotient_sign_matches_the_exact_product() {
    for x in edges() {
        for y in edges() {
            for d in edges().filter(|&d| d != 0) {
                assert_eq!(
                    quotient_is_negative(x, y, d),
                    quotient_negative(Wide::product(x, y), d),
                    "{x} * {y} / {d}"
                );
            }
        }
    }
}

#[test]
fn exact_widened_division_helpers_match_floor_and_ceiling() {
    let env = unmetered_env();
    let check = |x: i128, y: i128, d: i128| {
        let n = Wide::product(x, y);
        let product = I256::from_i128(&env, x).mul(&I256::from_i128(&env, y));
        let divisor = I256::from_i128(&env, d);
        // The general branch, then the shortcut the callers take when the
        // quotient is not negative.
        for nonneg in [false, !quotient_negative(n, d)] {
            let floor = div_floor_i256(&env, &product, &divisor, nonneg).to_i128();
            assert_eq!(floor.is_some(), floor_fits(n, d), "floor {x} * {y} / {d}");
            assert!(
                floor.is_none_or(|q| is_floor(q, n, d)),
                "floor {x} * {y} / {d} = {floor:?} (nonneg {nonneg})"
            );
            let ceil = div_ceil_i256(&env, &product, &divisor, nonneg).to_i128();
            assert_eq!(ceil.is_some(), ceil_fits(n, d), "ceil {x} * {y} / {d}");
            assert!(
                ceil.is_none_or(|q| is_ceil(q, n, d)),
                "ceil {x} * {y} / {d} = {ceil:?} (nonneg {nonneg})"
            );
        }
    };

    for x in edges() {
        for y in edges() {
            for d in edges().filter(|&d| d != 0) {
                check(x, y, d);
            }
        }
    }
    let mut sampler = Sampler::new(2);
    for _ in 0..RANDOM_CASES {
        let (x, y, d) = (sampler.signed(), sampler.signed(), sampler.signed());
        if d != 0 {
            check(x, y, d);
        }
    }
}

#[test]
fn exact_mul_div_matches_the_exact_quotient_or_reports_overflow() {
    let env = unmetered_env();
    // A typed panic gets slower as the environment accumulates host objects,
    // so each expected panic runs in its own empty environment.
    let check = |x: i128, y: i128, d: i128| {
        let n = Wide::product(x, y);
        let clamp = if quotient_negative(n, d) {
            i128::MIN
        } else {
            i128::MAX
        };

        if floor_fits(n, d) {
            let q = mul_div_floor(&env, x, y, d);
            assert!(is_floor(q, n, d), "floor {x} * {y} / {d} = {q}");
            assert_eq!(
                mul_div_floor_saturating(&env, x, y, d),
                q,
                "saturating floor {x} * {y} / {d}"
            );
        } else {
            let code = contract_error_code(|| {
                mul_div_floor(&Env::default(), x, y, d);
            });
            assert_eq!(code, MATH_OVERFLOW, "floor {x} * {y} / {d}");
            assert_eq!(
                mul_div_floor_saturating(&env, x, y, d),
                clamp,
                "saturating floor {x} * {y} / {d}"
            );
        }

        if ceil_fits(n, d) {
            let q = mul_div_ceil(&env, x, y, d);
            assert!(is_ceil(q, n, d), "ceil {x} * {y} / {d} = {q}");
        } else {
            let code = contract_error_code(|| {
                mul_div_ceil(&Env::default(), x, y, d);
            });
            assert_eq!(code, MATH_OVERFLOW, "ceil {x} * {y} / {d}");
        }

        let half_up = try_mul_div_half_up(&env, x, y, d);
        let in_domain = x >= 0 && y >= 0 && d > 0;
        if in_domain && half_up_fits(n, d) {
            assert!(
                half_up.is_some_and(|q| is_half_up(q, n, d)),
                "half up {x} * {y} / {d} = {half_up:?}"
            );
            assert_eq!(
                Some(mul_div_half_up(&env, x, y, d)),
                half_up,
                "half up {x} * {y} / {d}"
            );
        } else {
            assert_eq!(half_up, None, "half up {x} * {y} / {d}");
            // Outside the domain the panicking variant trips its debug
            // assertion first, so only the in-domain overflow is typed.
            if in_domain {
                let code = contract_error_code(|| {
                    mul_div_half_up(&Env::default(), x, y, d);
                });
                assert_eq!(code, MATH_OVERFLOW, "half up {x} * {y} / {d}");
            }
        }
    };

    for x in edges() {
        for y in edges() {
            for d in edges().filter(|&d| d != 0) {
                check(x, y, d);
            }
        }
    }
    let mut sampler = Sampler::new(3);
    for _ in 0..RANDOM_CASES {
        let (x, y, d) = (sampler.signed(), sampler.signed(), sampler.signed());
        if d != 0 {
            check(x, y, d);
        }
        check(
            sampler.non_negative(),
            sampler.non_negative(),
            sampler.non_negative().max(1),
        );
    }
}

#[test]
fn exact_div_by_int_rounds_halves_away_from_zero() {
    let env = Env::default();
    let check = |a: i128, b: i128| {
        let q = div_by_int_half_up(&env, a, b);
        assert!(is_half_away(q, Wide::from(a), b), "{a} / {b} = {q}");
    };

    for a in edges() {
        for b in edges().filter(|&b| b > 0) {
            check(a, b);
        }
    }
    let mut sampler = Sampler::new(4);
    for _ in 0..RANDOM_CASES {
        check(sampler.signed(), sampler.non_negative().max(1));
    }
}

#[test]
fn exact_rescale_matches_each_rounding_rule_or_reports_overflow() {
    // Protocol decimals, their neighbours, and gaps on both sides of the
    // largest power of ten that fits `i128` (10^38).
    const DECIMALS: [u32; 14] = [0, 1, 6, 7, 9, 18, 19, 27, 28, 37, 38, 39, 40, 77];
    type Rescale = fn(&Env, i128, u32, u32) -> i128;
    const VARIANTS: [(Rounding, Rescale); 3] = [
        (Rounding::HalfAway, rescale_half_up),
        (Rounding::TowardZero, rescale_floor),
        (Rounding::Ceil, rescale_ceil),
    ];

    let env = Env::default();
    let mut sampler = Sampler::new(5);
    for from in DECIMALS {
        for to in DECIMALS {
            for a in edges().chain(core::iter::repeat_with(|| sampler.signed()).take(24)) {
                for (rounding, rescale) in VARIANTS {
                    if rescale_fits(a, from, to) {
                        let r = rescale(&env, a, from, to);
                        assert!(
                            is_rescaled(r, a, from, to, rounding),
                            "{rounding:?} {a} {from}->{to} = {r}"
                        );
                    } else {
                        let code = contract_error_code(|| {
                            rescale(&env, a, from, to);
                        });
                        assert_eq!(code, MATH_OVERFLOW, "{rounding:?} {a} {from}->{to}");
                    }
                }
            }
        }
    }
}

/// Each reference predicate accepts exactly one integer: the neighbours of the
/// correct result fail, including at an exact half and beyond `i128` products.
#[test]
fn exact_reference_predicates_reject_the_neighbouring_integers() {
    let n = Wide::from;
    // -7 / 3 = -2.33..: the floor is -3 and the ceiling is -2.
    assert!(is_floor(-3, n(-7), 3) && !is_floor(-2, n(-7), 3) && !is_floor(-4, n(-7), 3));
    assert!(is_ceil(-2, n(-7), 3) && !is_ceil(-3, n(-7), 3) && !is_ceil(-1, n(-7), 3));
    // 7 / -3 is the same quotient through a negative divisor.
    assert!(is_floor(-3, n(7), -3) && !is_floor(-2, n(7), -3));
    assert!(is_ceil(-2, n(7), -3) && !is_ceil(-3, n(7), -3));
    // 5 / 2 is an exact half: half up gives 3, and -5 / 2 rounds away to -3.
    assert!(is_half_up(3, n(5), 2) && !is_half_up(2, n(5), 2));
    assert!(is_half_away(-3, n(-5), 2) && !is_half_away(-2, n(-5), 2));
    // 4 / 9 is just below a half and rounds down.
    assert!(is_half_up(0, n(4), 9) && !is_half_up(1, n(4), 9));
    // MAX * MAX / MAX is exactly MAX; dividing by MAX - 1 no longer fits.
    let square = Wide::product(i128::MAX, i128::MAX);
    assert!(is_floor(i128::MAX, square, i128::MAX));
    assert!(!is_floor(i128::MAX - 1, square, i128::MAX));
    assert!(floor_fits(square, i128::MAX) && !floor_fits(square, i128::MAX - 1));
    assert!(ceil_fits(square, i128::MAX) && !ceil_fits(square, i128::MAX - 1));
    assert!(half_up_fits(square, i128::MAX) && !half_up_fits(square, i128::MAX - 1));
}

/// The reference quotients find the integer their predicate accepts, report
/// `None` exactly when it does not fit, and agree with native division where
/// the product fits `i128`.
#[test]
fn exact_reference_quotients_match_their_predicates_and_native_division() {
    let mut sampler = Sampler::new(11);
    for _ in 0..RANDOM_CASES {
        let (x, y) = (sampler.signed(), sampler.signed());
        let d = sampler.non_negative().max(1);
        let n = Wide::product(x, y);

        let floor = floor_of(x, y, d);
        assert_eq!(floor.is_some(), floor_fits(n, d), "floor {x} * {y} / {d}");
        assert!(
            floor.is_none_or(|q| is_floor(q, n, d)),
            "floor {x} * {y} / {d}"
        );
        let ceil = ceil_of(x, y, d);
        assert_eq!(ceil.is_some(), ceil_fits(n, d), "ceil {x} * {y} / {d}");
        assert!(
            ceil.is_none_or(|q| is_ceil(q, n, d)),
            "ceil {x} * {y} / {d}"
        );
        if !n.is_negative() {
            let half_up = half_up_of(x, y, d);
            assert_eq!(
                half_up.is_some(),
                half_up_fits(n, d),
                "half up {x} * {y} / {d}"
            );
            assert!(
                half_up.is_none_or(|q| is_half_up(q, n, d)),
                "half up {x} * {y} / {d}"
            );
        }

        if let Some(p) = x.checked_mul(y) {
            assert_eq!(floor, Some(p.div_euclid(d)), "floor {x} * {y} / {d}");
            let rounds_up = i128::from(p.rem_euclid(d) != 0);
            assert_eq!(
                ceil,
                p.div_euclid(d).checked_add(rounds_up),
                "ceil {x} * {y} / {d}"
            );
        }
    }
    // 5 / 2 is an exact half and rounds up; 4 / 9 is below a half.
    assert_eq!(half_up_of(5, 1, 2), Some(3));
    assert_eq!(half_up_of(4, 1, 9), Some(0));
    // The largest quotient is found, and one unit more no longer fits.
    assert_eq!(floor_of(i128::MAX, i128::MAX, i128::MAX), Some(i128::MAX));
    assert_eq!(half_up_of(i128::MAX, i128::MAX, i128::MAX - 1), None);
    assert_eq!(floor_of(i128::MIN, 1, 1), Some(i128::MIN));
    assert_eq!(ceil_of(i128::MIN, 1, 1), Some(i128::MIN));
}
