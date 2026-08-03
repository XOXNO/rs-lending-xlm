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
#[should_panic(expected = "rescale_half_up upscale overflow")]
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
#[should_panic(expected = "downscale factor overflow")]
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
#[should_panic(expected = "rescale_floor upscale factor overflow")]
fn test_rescale_floor_upscale_factor_overflow_panics() {
    let _ = rescale_floor(1, 0, 39);
}

#[test]
#[should_panic(expected = "rescale_floor upscale overflow")]
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
#[should_panic(expected = "rescale_ceil upscale factor overflow")]
fn test_rescale_ceil_upscale_factor_overflow_panics() {
    let _ = rescale_ceil(1, 0, 39);
}

#[test]
#[should_panic(expected = "rescale_ceil upscale overflow")]
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
