use super::*;
use crate::math::test_support::{
    ceil_fits, ceil_of, floor_fits, floor_of, half_up_fits, half_up_of, is_ceil, is_floor,
    is_half_away, is_half_up, is_rescaled, rescale_fits, Rounding, Sampler, Wide,
};
use soroban_sdk::Env;

#[test]
fn typed_divisions_preserve_rounding_and_overflow_policy() {
    let env = Env::default();
    for (value, divisor) in [(1, 3), (i128::MAX, 1), (i128::MAX, RAY), (-3, 2)] {
        let amount = Ray::from(value);
        let index = Ray::from(divisor);
        assert_eq!(
            amount.div_floor_saturating(&env, index).raw(),
            fp_core::mul_div_floor_saturating(&env, value, RAY, divisor)
        );
        assert_eq!(
            amount.try_div(&env, index).map(Ray::raw),
            fp_core::try_mul_div_half_up(&env, value, RAY, divisor)
        );
    }
    for (value, divisor) in [(1, 3), (i128::MAX, WAD), (-3, 2)] {
        assert_eq!(
            Wad::from(value).div_ceil(&env, Wad::from(divisor)).raw(),
            fp_core::mul_div_ceil(&env, value, WAD, divisor)
        );
    }
    assert_eq!(Ray::ONE.try_div(&env, Ray::ZERO), None);
    assert_eq!(Ray::from(i128::MAX).try_div(&env, Ray::from(1)), None);
    for (value, index) in [
        (1, 3),
        (i128::MAX, RAY),
        (i128::MAX, RAY + 1),
        (-3, 2),
        (3, -2),
    ] {
        assert_eq!(
            Ray::from(value)
                .try_mul(&env, Ray::from(index))
                .map(Ray::raw),
            fp_core::try_mul_div_half_up(&env, value, index, RAY)
        );
    }
    assert_eq!(
        Ray::from(i128::MAX).try_mul(&env, Ray::ONE),
        Some(Ray::from(i128::MAX))
    );
    assert_eq!(Ray::from(i128::MAX).try_mul(&env, Ray::from(RAY + 1)), None);
    assert_eq!(Ray::from(-1i128).try_mul(&env, Ray::ONE), None);
    assert_eq!(
        Wad::from(i128::MAX)
            .mul_floor_saturating(&env, Wad::from(2 * WAD))
            .raw(),
        i128::MAX
    );
}

#[test]
fn token_valuation_does_not_normalize_or_round_the_amount_first() {
    let env = Env::default();
    assert_eq!(
        Wad::from(1).try_value_of_token(&env, i128::MAX, 0),
        Some(Wad::from(i128::MAX))
    );
    assert_eq!(
        Wad::from(5).try_value_of_token(&env, 1, 1),
        Some(Wad::from(1))
    );
    assert_eq!(Wad::ONE.try_value_of_token(&env, -1, 7), None);
    assert_eq!(Wad::from(-1).try_value_of_token(&env, 1, 7), None);
    assert_eq!(Wad::ONE.try_value_of_token(&env, 0, 39), None);
    assert_eq!(Wad::try_from_token_upscale(-1, 18), None);
    assert_eq!(Wad::try_from_token_upscale(0, 19), None);
    assert_eq!(
        Wad::try_from_token_upscale(i128::MAX, 18),
        Some(Wad::from(i128::MAX))
    );
    assert_eq!(Wad::try_from_token_upscale(i128::MAX, 17), None);
}

#[test]
fn proportional_scaling_rounds_once_in_the_output_unit() {
    let env = Env::default();
    // Quantizing 1/3 to WAD first would turn this exact result into zero.
    assert_eq!(
        Ray::from(3).mul_wad_ratio_floor(&env, Wad::from(1), Wad::from(3)),
        Ray::from(1)
    );
    assert_eq!(Wad::from(3).mul_ratio_floor(&env, 1, 3), Wad::from(1));
    assert_eq!(
        Wad::from(1).apply_ratio_to_amount_floor(&env, Wad::from(3), 3),
        1
    );
    assert_eq!(Wad::from(-3).mul_ratio_floor(&env, 1, 2), Wad::from(-2));
    assert_eq!(
        Ray::from(i128::MAX).mul_wad_ratio_floor(&env, Wad::from(3), Wad::from(3)),
        Ray::from(i128::MAX)
    );
}

#[test]
fn bps_ratios_and_fees_keep_their_rounding_direction() {
    let env = Env::default();
    let numerator = Wad::from(1);
    let denominator = Wad::from(3);
    assert_eq!(
        Bps::from_fraction_floor(&env, numerator, denominator).raw(),
        3_333
    );
    assert_eq!(
        Bps::from_fraction_ceil(&env, numerator, denominator).raw(),
        3_334
    );
    assert_eq!(
        Bps::try_from_fraction(&env, numerator, denominator),
        Some(Bps::from(3_333))
    );
    assert_eq!(Bps::try_from_fraction(&env, numerator, Wad::ZERO), None);
    assert_eq!(
        Bps::try_from_fraction(&env, Wad::from(i128::MAX), numerator),
        None
    );
    assert_eq!(Bps::ONE.div(&env, Bps::from(10_500)).raw(), 9_524);
    assert_eq!(Bps::from(1).apply_to_ceil(&env, 1), 1);
    assert_eq!(
        Bps::from(1).apply_to_ray_ceil(&env, Ray::from(1)),
        Ray::from(1)
    );
    assert_eq!(Bps::from_weighted_sum(&env, 5, Wad::from(2)).raw(), 3);
}

#[test]
fn integer_rate_multiplication_preserves_boundaries() {
    let env = Env::default();
    assert_eq!(Ray::from(i128::MAX).mul_int(&env, 1), Ray::from(i128::MAX));
    assert_eq!(Ray::from(i128::MAX).mul_int(&env, 0), Ray::ZERO);
}

#[test]
#[should_panic(expected = "Error(Contract, #33)")]
fn integer_rate_multiplication_keeps_overflow_error() {
    let env = Env::default();
    let _ = Ray::from(i128::MAX).mul_int(&env, 2);
}

#[test]
#[should_panic(expected = "Error(Contract, #55)")]
fn saturating_division_keeps_zero_divisor_error_priority() {
    let env = Env::default();
    let _ = Ray::from(-1).div_floor_saturating(&env, Ray::ZERO);
}

#[test]
#[should_panic(expected = "Error(Contract, #33)")]
fn wad_ceiling_division_keeps_overflow_error() {
    let env = Env::default();
    let _ = Wad::from(i128::MAX).div_ceil(&env, Wad::from(1));
}

#[test]
fn risk_bound_comparison_keeps_the_exact_product_boundary() {
    let env = Env::default();
    for (threshold, bonus) in [
        (10_000, 0),
        (9_999, 1),
        (9_999, 2),
        (6_666, 5_000),
        (6_667, 5_000),
        (1, u32::MAX),
    ] {
        let threshold = i128::from(threshold);
        let bonus = i128::from(bonus);
        assert_eq!(
            Bps::from(BPS + bonus).apply_to_ceil(&env, threshold) <= BPS,
            threshold * (BPS + bonus) <= BPS * BPS
        );
    }
}

#[test]
fn test_ray_mul_div() {
    let env = Env::default();
    let a = Ray::from(2 * RAY);
    let b = Ray::from(3 * RAY);
    assert_eq!(a.mul(&env, b), Ray::from(6 * RAY));

    let c = Ray::from(6 * RAY);
    let d = Ray::from(3 * RAY);
    assert_eq!(c.div(&env, d), Ray::from(2 * RAY));
}

#[test]
fn test_ray_add_sub() {
    let env = Env::default();
    let a = Ray::from(RAY);
    let b = Ray::from(RAY / 2);
    assert_eq!(a.checked_add(&env, b).raw(), RAY + RAY / 2);
    assert_eq!(a.checked_sub(&env, b).raw(), RAY / 2);
}

#[test]
#[should_panic(expected = "Error(Contract, #33)")]
fn test_ray_add_overflow_panics() {
    let env = Env::default();
    let _ = Ray::from(i128::MAX).checked_add(&env, Ray::from(1));
}

#[test]
fn test_ray_div_by_int() {
    let env = Env::default();
    let x = Ray::from(7);
    assert_eq!(x.div_by_int(&env, 2).raw(), 4);
}

#[test]
fn test_ray_to_wad() {
    let env = Env::default();
    let r = Ray::from(RAY);
    let w = r.to_wad(&env);
    assert_eq!(w.raw(), WAD);
}

#[test]
fn test_ray_div_floor() {
    let env = Env::default();
    let amount = Ray::from(105 * RAY / 100);
    let ratio = Ray::from(11 * RAY / 10);

    assert_eq!(
        amount.div_floor(&env, ratio).raw(),
        954_545_454_545_454_545_454_545_454
    );
}

#[test]
fn test_ray_div_ceil() {
    let env = Env::default();
    let amount = Ray::from(2);
    let index = Ray::from(3 * RAY);

    assert_eq!(amount.div_floor(&env, index).raw(), 0);
    assert_eq!(amount.div_ceil(&env, index).raw(), 1);
}

#[test]
fn test_ray_from_asset() {
    let env = Env::default();
    let r = Ray::from_asset(&env, 10_000_000, 7);
    assert_eq!(r.raw(), RAY);
}

#[test]
fn test_ray_to_asset() {
    let env = Env::default();
    let r = Ray::from(RAY);
    assert_eq!(r.to_asset(&env, 7), 10_000_000);
}

#[test]
fn test_ray_asset_roundtrip() {
    let env = Env::default();
    let original = 12_345_678;
    let ray = Ray::from_asset(&env, original, 7);
    assert_eq!(ray.to_asset(&env, 7), original);
}

#[test]
fn test_wad_mul_div() {
    let env = Env::default();
    let price = Wad::from(2 * WAD);
    let amount = Wad::from(3 * WAD);
    assert_eq!(amount.mul(&env, price), Wad::from(6 * WAD));

    let total = Wad::from(6 * WAD);
    let divisor = Wad::from(3 * WAD);
    assert_eq!(total.div(&env, divisor), Wad::from(2 * WAD));
}

#[test]
fn test_wad_try_mul_matches_mul_when_in_range() {
    let env = Env::default();
    let a = Wad::from(2 * WAD);
    let b = Wad::from(3 * WAD);
    assert_eq!(a.try_mul(&env, b), Some(a.mul(&env, b)));
}

#[test]
fn test_wad_try_mul_softens_overflow() {
    let env = Env::default();
    let huge = Wad::from(i128::MAX);
    assert_eq!(huge.try_mul(&env, huge), None);
}

#[test]
fn test_wad_div_floor_saturating() {
    let env = Env::default();

    let total = Wad::from(6 * WAD);
    let divisor = Wad::from(3 * WAD);
    assert_eq!(
        total.div_floor_saturating(&env, divisor),
        Wad::from(2 * WAD)
    );

    let large = Wad::from(i128::MAX / 2);
    let tiny = Wad::from(1);
    assert_eq!(large.div_floor_saturating(&env, tiny), Wad::from(i128::MAX));
}

#[test]
fn test_wad_from_token() {
    let env = Env::default();
    let w = Wad::from_token(&env, 1_000_000, 6);
    assert_eq!(w.raw(), 1_000_000_000_000_000_000);
}

#[test]
fn test_wad_to_token() {
    let env = Env::default();
    let w = Wad::from(WAD);
    assert_eq!(w.to_token(&env, 6), 1_000_000);
    assert_eq!(w.to_token(&env, 7), 10_000_000);
}

#[test]
fn test_wad_to_ray() {
    let env = Env::default();
    let w = Wad::from(WAD);
    assert_eq!(w.to_ray(&env).raw(), RAY);
}

#[test]
fn test_wad_min_max() {
    let a = Wad::from(10);
    let b = Wad::from(20);
    assert_eq!(a.min(b), a);
    assert_eq!(a.max(b), b);
}

#[test]
fn test_bps_to_wad() {
    let env = Env::default();
    let ltv = Bps::from(8000);
    let w = ltv.to_wad(&env);
    assert_eq!(w.raw(), 800_000_000_000_000_000);
}

#[test]
fn test_bps_apply_to() {
    let env = Env::default();
    let fee_bps = Bps::from(50);
    let amount = 1_000_000_000;
    let fee = fee_bps.apply_to(&env, amount);
    assert_eq!(fee, 5_000_000);
}

#[test]
fn test_bps_flash_loan_fee_on_charges_min_unit_when_bps_positive() {
    let env = Env::default();
    let fee_bps = Bps::from(9);
    assert_eq!(fee_bps.apply_to(&env, 1), 0);
    assert_eq!(fee_bps.flash_loan_fee_on(&env, 1), 1);
}

#[test]
fn test_bps_flash_loan_fee_on_allows_zero_when_bps_zero() {
    let env = Env::default();
    assert_eq!(Bps::from(0).flash_loan_fee_on(&env, 1), 0);
}

#[test]
fn test_bps_apply_to_wad() {
    let env = Env::default();
    let threshold = Bps::from(8000);
    let value = Wad::from(100 * WAD);
    let weighted = threshold.apply_to_wad(&env, value);
    assert_eq!(weighted.raw(), 80 * WAD);
}

#[test]
fn test_bps_apply_to_ray() {
    let env = Env::default();
    let fee = Bps::from(250).apply_to_ray(&env, Ray::from(4 * RAY));
    assert_eq!(fee.raw(), RAY / 10);
}

#[test]
#[should_panic(expected = "Error(Contract, #33)")]
fn test_bps_add_overflow_panics() {
    let env = Env::default();
    let _ = Bps::from(i128::MAX).checked_add(&env, Bps::from(1));
}

#[test]
fn test_ray_one_plus_compound() {
    let env = Env::default();
    let x = Ray::from(RAY / 10);
    let term2 = Ray::from(RAY / 200);
    let result = Ray::ONE.checked_add(&env, x).checked_add(&env, term2);
    assert_eq!(result.raw(), RAY + RAY / 10 + RAY / 200);
}

#[test]
fn test_ordering() {
    assert!(Ray::ZERO < Ray::ONE);
    assert!(Wad::ZERO < Wad::ONE);
    assert!(Bps::from(5000) < Bps::ONE);
}

#[test]
fn test_ray_checked_add() {
    let env = Env::default();
    let mut x = Ray::from(RAY);
    x = x.checked_add(&env, Ray::from(RAY / 2));
    assert_eq!(x.raw(), RAY + RAY / 2);
}

#[test]
fn test_ray_checked_sub_value() {
    let env = Env::default();
    let mut x = Ray::from(RAY);
    x = x.checked_sub(&env, Ray::from(RAY / 4));
    assert_eq!(x.raw(), RAY - RAY / 4);
}

#[test]
fn test_ray_checked_sub_ok() {
    let env = Env::default();
    let a = Ray::from(10 * RAY);
    let b = Ray::from(3 * RAY);
    assert_eq!(a.checked_sub(&env, b).raw(), 7 * RAY);
}

#[test]
#[should_panic]
fn test_ray_checked_sub_underflow_panics() {
    let env = Env::default();
    let a = Ray::from(3 * RAY);
    let b = Ray::from(10 * RAY);
    let _ = a.checked_sub(&env, b);
}

#[test]
#[should_panic]
fn test_ray_checked_sub_rejects_negative_self() {
    let env = Env::default();
    let _ = Ray::from(-1).checked_sub(&env, Ray::from(0));
}

#[test]
fn test_ray_checked_sub_chain() {
    let env = Env::default();
    let mut x = Ray::from(10 * RAY);
    x = x.checked_sub(&env, Ray::from(4 * RAY));
    assert_eq!(x.raw(), 6 * RAY);
}

#[test]
fn test_ray_from_asset_high_decimals() {
    let env = Env::default();
    let r = Ray::from_asset(&env, 1, 0);
    assert_eq!(r.raw(), RAY);
}

#[test]
fn test_wad_checked_sub() {
    let env = Env::default();
    let mut w = Wad::from(WAD);
    w = w.checked_sub(&env, Wad::from(WAD / 3));
    assert_eq!(w.raw(), WAD - WAD / 3);
}

#[test]
fn test_wad_max_chooses_other_when_self_smaller() {
    let a = Wad::from(1);
    let b = Wad::from(2);
    assert_eq!(a.max(b), b);
}

#[test]
fn test_wad_min_chooses_other_when_self_larger() {
    let a = Wad::from(10);
    let b = Wad::from(5);
    assert_eq!(a.min(b), b);
}

#[test]
fn test_wad_div_floor_rounds_down() {
    let env = Env::default();
    let a = Wad::from(2 * WAD);
    let b = Wad::from(3 * WAD);
    let half_up = a.div(&env, b).raw();
    let floor = a.div_floor(&env, b).raw();
    assert!(
        floor < half_up,
        "div_floor must round strictly down for 2/3"
    );
}

#[test]
fn test_bps_checked_add_rebinding() {
    let env = Env::default();
    let mut b = Bps::from(5000);
    b = b.checked_add(&env, Bps::from(2000));
    assert_eq!(b.raw(), 7000);
}

#[test]
fn test_bps_checked_sub_rebinding() {
    let env = Env::default();
    let mut b = Bps::from(5000);
    b = b.checked_sub(&env, Bps::from(1500));
    assert_eq!(b.raw(), 3500);
}

#[test]
fn test_bps_sub() {
    let env = Env::default();
    let a = Bps::from(7500);
    let b = Bps::from(2500);
    assert_eq!(a.checked_sub(&env, b).raw(), 5000);
}

#[test]
fn test_ray_mul_exact_products() {
    let env = Env::default();

    let half = Ray::from(RAY / 2);
    let one = Ray::ONE;
    assert_eq!(half.mul(&env, one).raw(), RAY / 2);

    assert_eq!(half.mul(&env, half).raw(), RAY / 4);
}

#[test]
#[should_panic]
fn test_ray_div_by_zero_panics() {
    let env = Env::default();
    let _ = Ray::ONE.div(&env, Ray::ZERO);
}

#[test]
#[should_panic]
fn test_ray_mul_overflow_panics() {
    let env = Env::default();
    let _ = Ray::from(i128::MAX).mul(&env, Ray::from(i128::MAX));
}

#[test]
fn test_ray_checked_sub_equal_returns_zero() {
    let env = Env::default();
    let a = Ray::from(123 * RAY);
    assert_eq!(a.checked_sub(&env, a), Ray::ZERO);
}

#[test]
fn test_ray_from_asset_at_ray_decimals_is_identity() {
    let env = Env::default();
    let r = Ray::from_asset(&env, 12345, 27);
    assert_eq!(r.raw(), 12345);
}

#[test]
#[should_panic(expected = "Error(Contract, #33)")]
fn test_ray_sub_panics_on_negative_result() {
    let env = Env::default();
    let a = Ray::from(RAY);
    let b = Ray::from(2 * RAY);
    let _ = a.checked_sub(&env, b);
}

#[test]
#[should_panic]
fn test_wad_div_by_zero_panics() {
    let env = Env::default();
    let _ = Wad::ONE.div(&env, Wad::ZERO);
}

#[test]
fn test_wad_mul_no_rounding_when_exact() {
    let env = Env::default();
    let half = Wad::from(WAD / 2);
    assert_eq!(half.mul(&env, half).raw(), WAD / 4);
}

#[test]
#[should_panic(expected = "Error(Contract, #33)")]
fn test_wad_sub_panics_on_negative_result() {
    let env = Env::default();
    let a = Wad::from(WAD);
    let b = Wad::from(3 * WAD);
    let _ = a.checked_sub(&env, b);
}

#[test]
fn test_wad_min_max_equal_operands() {
    let a = Wad::from(42);
    let b = Wad::from(42);

    assert_eq!(a.min(b), b);
    assert_eq!(a.max(b), b);
}

#[test]
fn test_wad_max_returns_self_when_strictly_greater() {
    let a = Wad::from(100);
    let b = Wad::from(10);
    assert_eq!(a.max(b), a);
    assert_eq!(b.min(a), b);
}

#[test]
fn test_ray_checked_add_ok() {
    let env = Env::default();
    let a = Ray::from(RAY);
    let b = Ray::from(RAY / 2);
    assert_eq!(a.checked_add(&env, b).raw(), RAY + RAY / 2);
}

#[test]
#[should_panic]
fn test_ray_checked_add_overflow_panics() {
    let env = Env::default();
    let _ = Ray::from(i128::MAX).checked_add(&env, Ray::from(1));
}

#[test]
fn test_wad_checked_add_ok() {
    let env = Env::default();
    assert_eq!(
        Wad::from(WAD).checked_add(&env, Wad::from(WAD)).raw(),
        2 * WAD
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #33)")]
fn test_wad_checked_add_overflow_panics() {
    let env = Env::default();
    let _ = Wad::from(i128::MAX).checked_add(&env, Wad::from(1));
}

#[test]
fn test_wad_checked_sub_ok() {
    let env = Env::default();
    let a = Wad::from(3 * WAD);
    let b = Wad::from(WAD);
    assert_eq!(a.checked_sub(&env, b).raw(), 2 * WAD);
}

#[test]
#[should_panic]
fn test_wad_checked_sub_underflow_panics() {
    let env = Env::default();
    let _ = Wad::from(1).checked_sub(&env, Wad::from(2));
}

#[test]
fn test_bps_checked_add_ok() {
    let env = Env::default();
    let a = Bps::from(5_000);
    let b = Bps::from(2_500);
    assert_eq!(a.checked_add(&env, b).raw(), 7_500);
}

#[test]
#[should_panic]
fn test_bps_checked_add_overflow_panics() {
    let env = Env::default();
    let _ = Bps::from(i128::MAX).checked_add(&env, Bps::from(1));
}

#[test]
fn test_bps_checked_sub_ok() {
    let env = Env::default();
    let a = Bps::from(7_500);
    let b = Bps::from(2_500);
    assert_eq!(a.checked_sub(&env, b).raw(), 5_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #33)")]
fn test_bps_checked_sub_underflow_panics() {
    let env = Env::default();
    let _ = Bps::from(100).checked_sub(&env, Bps::from(500));
}

#[test]
fn test_wad_from_token_at_wad_decimals_is_identity() {
    let env = Env::default();
    let w = Wad::from_token(&env, 98765, 18);
    assert_eq!(w.raw(), 98765);
}

#[test]
fn test_wad_to_token_half_unit_rounds_up() {
    let env = Env::default();
    let half = Wad::from(1_500_000_000_000i128);
    assert_eq!(half.to_token(&env, 6), 2);
}

#[test]
fn test_bps_apply_to_boundaries() {
    let env = Env::default();
    let amount = 1_000_000i128;
    assert_eq!(Bps::from(0).apply_to(&env, amount), 0);
    assert_eq!(Bps::ONE.apply_to(&env, amount), amount);
}

#[test]
fn test_bps_apply_to_wad_and_ray_at_one_returns_input() {
    let env = Env::default();
    let w = Wad::from(123 * WAD);
    let r = Ray::from(456 * RAY);
    assert_eq!(Bps::ONE.apply_to_wad(&env, w).raw(), w.raw());
    assert_eq!(Bps::ONE.apply_to_ray(&env, r).raw(), r.raw());
}

#[test]
fn test_bps_to_wad_above_one_does_not_panic() {
    let env = Env::default();

    assert_eq!(Bps::from(20_000).to_wad(&env).raw(), 2 * WAD);
}

#[test]
fn test_ray_div_by_int_negative_rounds_away_from_zero() {
    let env = Env::default();
    let x = Ray::from(-7);
    assert_eq!(x.div_by_int(&env, 2).raw(), -4);
}

#[test]
fn test_ray_div_floor_vs_div_diverges_on_half_remainder() {
    let env = Env::default();
    let a = Ray::from(2 * RAY);
    let b = Ray::from(3 * RAY);
    let half_up = a.div(&env, b).raw();
    let floor = a.div_floor(&env, b).raw();

    assert_eq!(
        half_up - floor,
        1,
        "div and div_floor must differ by 1 ulp on a half-remainder"
    );
}

#[test]
fn test_ray_checked_sub_zero_zero_returns_zero() {
    let env = Env::default();
    assert_eq!(Ray::ZERO.checked_sub(&env, Ray::ZERO), Ray::ZERO);
}

#[test]
fn test_wad_checked_sub_zero_zero_returns_zero() {
    let env = Env::default();
    assert_eq!(Wad::ZERO.checked_sub(&env, Wad::ZERO), Wad::ZERO);
}

#[test]
fn test_bps_checked_sub_zero_zero_returns_zero() {
    let env = Env::default();
    let zero = Bps::from(0i128);
    assert_eq!(zero.checked_sub(&env, zero), zero);
}

#[test]
#[should_panic]
fn test_ray_checked_sub_negative_rhs_with_zero_self_panics() {
    let env = Env::default();
    let _ = Ray::from(0).checked_sub(&env, Ray::from(-1));
}

#[test]
#[should_panic]
fn test_wad_checked_sub_negative_rhs_with_zero_self_panics() {
    let env = Env::default();
    let _ = Wad::from(0).checked_sub(&env, Wad::from(-1));
}

#[test]
#[should_panic]
fn test_bps_checked_sub_negative_rhs_with_zero_self_panics() {
    let env = Env::default();
    let _ = Bps::from(0i128).checked_sub(&env, Bps::from(-1i128));
}

#[test]
fn test_wad_sub_equal_returns_zero() {
    let env = Env::default();
    assert_eq!(Wad::ONE.checked_sub(&env, Wad::ONE), Wad::ZERO);
}

#[test]
fn test_bps_sub_equal_returns_zero() {
    let env = Env::default();
    assert_eq!(Bps::ONE.checked_sub(&env, Bps::ONE), Bps::from(0i128));
}

#[test]
fn test_ray_checked_add_returns_sum() {
    let env = Env::default();
    let mut x = Ray::from(RAY);
    x = x.checked_add(&env, Ray::from(2 * RAY));
    assert_eq!(x.raw(), 3 * RAY);
}

#[test]
fn test_wad_checked_add_returns_sum() {
    let env = Env::default();
    let mut x = Wad::from(crate::constants::WAD);
    x = x.checked_add(&env, Wad::from(2 * crate::constants::WAD));
    assert_eq!(x.raw(), 3 * crate::constants::WAD);
}

#[test]
fn test_wad_checked_sub_returns_difference() {
    let env = Env::default();
    let mut x = Wad::from(5 * crate::constants::WAD);
    x = x.checked_sub(&env, Wad::from(2 * crate::constants::WAD));
    assert_eq!(x.raw(), 3 * crate::constants::WAD);
}

#[test]
fn test_ray_to_asset_floor_pins_concrete_output() {
    let env = Env::default();
    let r = Ray::from(RAY + RAY / 2);
    assert_eq!(r.to_asset_floor(&env, 7), 15_000_000);

    let r2 = Ray::from(RAY + RAY * 999_999 / 1_000_000);
    assert_eq!(r2.to_asset_floor(&env, 0), 1);
}

#[test]
fn test_ray_to_asset_ceil_pins_concrete_output() {
    let env = Env::default();
    let r = Ray::from(RAY + RAY / 2);
    assert_eq!(r.to_asset_ceil(&env, 0), 2);

    assert_eq!(Ray::ONE.to_asset_ceil(&env, 7), 10_000_000);
}

#[test]
fn test_ray_mul_ceil_vs_floor_brackets_half_up() {
    let env = Env::default();

    let ulp = Ray::from(1);
    assert_eq!(ulp.mul_floor(&env, ulp).raw(), 0);
    assert_eq!(ulp.mul(&env, ulp).raw(), 0);
    assert_eq!(ulp.mul_ceil(&env, ulp).raw(), 1);

    let exact = Ray::from(RAY / 2);
    assert_eq!(
        exact.mul_floor(&env, Ray::ONE).raw(),
        exact.mul_ceil(&env, Ray::ONE).raw()
    );
}

#[test]
fn test_ray_to_wad_floor_and_ceil() {
    let env = Env::default();
    let r = Ray::from(RAY + 1);
    assert_eq!(r.to_wad_floor(&env).raw(), WAD);
    assert_eq!(r.to_wad_ceil(&env).raw(), WAD + 1);

    assert_eq!(Ray::ONE.to_wad_floor(&env).raw(), WAD);
    assert_eq!(Ray::ONE.to_wad_ceil(&env).raw(), WAD);
}

#[test]
fn test_wad_mul_floor_and_ceil_bracket_half_up() {
    let env = Env::default();

    let wei = Wad::from(1);
    assert_eq!(wei.mul_floor(&env, wei).raw(), 0);
    assert_eq!(wei.mul(&env, wei).raw(), 0);
    assert_eq!(wei.mul_ceil(&env, wei).raw(), 1);

    let a = Wad::from(WAD / 3);
    let floor = a.mul_floor(&env, a).raw();
    let half_up = a.mul(&env, a).raw();
    let ceil = a.mul_ceil(&env, a).raw();
    assert!(floor <= half_up && half_up <= ceil);
    assert_eq!(ceil - floor, 1);
}

#[test]
fn test_wad_to_token_floor_rounds_down() {
    let env = Env::default();
    let w = Wad::from(1_999_999_500_000_000_000i128);
    assert_eq!(w.to_token(&env, 6), 2_000_000);
    assert_eq!(w.to_token_floor(&env, 6), 1_999_999);

    assert_eq!(Wad::ONE.to_token_floor(&env, 6), 1_000_000);
}

#[test]
fn test_bps_apply_to_wad_floor_rounds_down() {
    let env = Env::default();

    let v = Wad::from(5);
    let half = Bps::from(5_000);

    assert_eq!(half.apply_to_wad(&env, v).raw(), 3);
    assert_eq!(half.apply_to_wad_floor(&env, v).raw(), 2);

    let exact = Wad::from(100 * WAD);
    assert_eq!(
        Bps::from(8_000).apply_to_wad_floor(&env, exact).raw(),
        80 * WAD
    );
}

#[test]
fn test_ray_from_fraction_builds_ratio() {
    let env = Env::default();

    assert_eq!(Ray::from_fraction(&env, 1, 4).raw(), RAY / 4);
}

#[test]
fn test_wad_add_checked() {
    let env = Env::default();
    let sum = Wad::from(WAD).checked_add(&env, Wad::from(WAD / 2));
    assert_eq!(sum.raw(), WAD + WAD / 2);
}

// --- largest representable balance (`docs/reference/formulas.md#numeric-limits`) --
//
// Every token amount enters the accounting through `Ray::from_asset`, which
// upscales by 10^(27 - decimals). That single multiplication is the ceiling on
// any balance the protocol can hold, and it is the same ceiling
// `common::validation::max_cap_for_decimals` computes for caps.

/// The largest asset-unit amount whose ray form still fits `i128`.
fn max_representable_units(decimals: u32) -> i128 {
    i128::MAX / 10i128.pow(RAY_DECIMALS - decimals)
}

#[test]
fn test_ray_from_asset_ceiling_holds_across_the_listable_decimal_range() {
    let env = Env::default();
    // MIN_ASSET_DECIMALS ..= MAX_ASSET_DECIMALS is 3 ..= 18.
    for decimals in 3u32..=18 {
        let ceiling = max_representable_units(decimals);
        let ray = Ray::from_asset(&env, ceiling, decimals);

        assert!(
            ray.raw() > 0,
            "ceiling must be representable at {decimals} decimals"
        );
        // One more asset unit would not fit: the residue below i128::MAX is
        // strictly smaller than one unit's worth of ray.
        let unit_in_ray = 10i128.pow(RAY_DECIMALS - decimals);
        assert!(
            i128::MAX - ray.raw() < unit_in_ray,
            "ceiling is not tight at {decimals} decimals",
        );
        assert_eq!(
            ray.to_asset(&env, decimals),
            ceiling,
            "roundtrip at the ceiling"
        );
    }
}

#[test]
fn test_balance_ceiling_is_the_same_whole_token_count_at_every_decimals() {
    // i128::MAX / RAY = 170_141_183_460.469…, so the ceiling is ~170.14 billion
    // whole tokens regardless of the asset's decimals.
    for decimals in 3u32..=18 {
        let whole_tokens = max_representable_units(decimals) / 10i128.pow(decimals);
        assert_eq!(
            whole_tokens, 170_141_183_460,
            "whole-token ceiling drifted at {decimals} decimals",
        );
    }
}

#[test]
#[should_panic(expected = "Error(Contract, #33)")]
fn test_ray_from_asset_one_unit_above_the_ceiling_overflows_at_min_decimals() {
    let env = Env::default();
    let _ = Ray::from_asset(&env, max_representable_units(3) + 1, 3);
}

#[test]
#[should_panic(expected = "Error(Contract, #33)")]
fn test_ray_from_asset_one_unit_above_the_ceiling_overflows_at_max_decimals() {
    let env = Env::default();
    let _ = Ray::from_asset(&env, max_representable_units(18) + 1, 18);
}

/// Real-execution check of the Certora rule `ray_asset_roundtrip_preserves_7_decimal_amount`
/// (`certora/common/spec/math_rules.rs`) over its assumed range. Upscale multiplies by
/// exactly `10^20` and downscale divides by it, so the roundtrip is exact;
/// `1e15 * 1e20 = 1e35` fits in i128.
#[test]
fn ray_asset_roundtrip_is_exact_over_the_certora_range() {
    let env = Env::default();
    const MAX: i128 = 1_000_000_000_000_000; // the rule's cvlr_assume upper bound

    let check = |amount: i128| {
        let back = Ray::from_asset(&env, amount, 7).to_asset(&env, 7);
        assert_eq!(
            back, amount,
            "Ray asset roundtrip lost value: {amount} -> {back}"
        );
    };

    for amount in [0i128, 1, 2, 9, 10, MAX - 1, MAX] {
        check(amount);
    }
    // Every power of ten in range, and the values either side of it.
    let mut p = 10i128;
    while p <= MAX {
        check(p - 1);
        check(p);
        if p < MAX {
            check(p + 1);
        }
        p *= 10;
    }
    // Values at and next to a half, where a wrong factor first changes the half-up result.
    for amount in [
        499_999i128,
        500_000,
        500_001,
        1_499_999,
        1_500_000,
        1_500_001,
    ] {
        check(amount);
    }
}

/// Real-execution check of the Certora rule `wad_token_roundtrip_preserves_7_decimal_amount`:
/// the same shape with the factor `10^11`.
#[test]
fn wad_token_roundtrip_is_exact_over_the_certora_range() {
    let env = Env::default();
    const MAX: i128 = 1_000_000_000_000_000;

    let check = |amount: i128| {
        let back = Wad::from_token(&env, amount, 7).to_token(&env, 7);
        assert_eq!(
            back, amount,
            "Wad token roundtrip lost value: {amount} -> {back}"
        );
    };

    for amount in [0i128, 1, 2, 9, 10, MAX - 1, MAX] {
        check(amount);
    }
    let mut p = 10i128;
    while p <= MAX {
        check(p - 1);
        check(p);
        if p < MAX {
            check(p + 1);
        }
        p *= 10;
    }
}

/// The Ray roundtrip is exact at every decimals value from 0 to 18. Upscaling from `d`
/// decimals multiplies by `10^(27-d)`, so the amount range shrinks as `d` falls.
#[test]
fn ray_asset_roundtrip_is_exact_across_decimals() {
    let env = Env::default();
    for decimals in 0u32..=18 {
        let headroom = 27u32 - decimals.min(27);
        // Keep amount * 10^headroom inside i128 (~1.7e38), with a decade spare.
        let exp = 37u32.saturating_sub(headroom).min(30);
        let max = 10i128.pow(exp);
        for amount in [0i128, 1, 7, max / 3, max - 1, max] {
            let back = Ray::from_asset(&env, amount, decimals).to_asset(&env, decimals);
            assert_eq!(
                back, amount,
                "Ray asset roundtrip lost value at {decimals} decimals: {amount} -> {back}"
            );
        }
    }
}

/// Pins the powers of ten that `fp_core::rescale` computes with `10i128.checked_pow` as
/// upscale factors.
#[test]
fn upscale_factors_are_exact_powers_of_ten() {
    assert_eq!(
        10i128.pow(11),
        100_000_000_000,
        "WAD factor from 7 decimals"
    );
    assert_eq!(
        10i128.pow(20),
        100_000_000_000_000_000_000,
        "RAY factor from 7 decimals"
    );
    // The largest factor the protocol can ask for: 0-decimal asset up to RAY.
    assert_eq!(
        10i128.pow(27),
        1_000_000_000_000_000_000_000_000_000,
        "RAY factor from 0 decimals"
    );
}

/// `Ray::mul_ratio_ceil` rounds up. `burn_claimable_revenue`
/// (`contracts/pool/src/cache/shares.rs`) uses it to burn at least the pro-rata revenue
/// shares on a partial claim; flooring would let repeated partial claims take unbacked cash.
#[test]
fn mul_ratio_ceil_rounds_up_and_never_below_the_exact_ratio() {
    let env = Env::default();
    assert_eq!(
        Ray::from(100i128).mul_ratio_ceil(&env, 1, 2).raw(),
        50,
        "exact ratio must not be rounded up"
    );
    // Inexact: must round up, not truncate.
    assert_eq!(Ray::from(100i128).mul_ratio_ceil(&env, 1, 3).raw(), 34);
    assert_eq!(Ray::from(10i128).mul_ratio_ceil(&env, 1, 4).raw(), 3);
    // A positive numerator never rounds to zero; `burn_claimable_revenue` asserts a
    // non-zero burn.
    assert_eq!(Ray::from(1i128).mul_ratio_ceil(&env, 1, i128::MAX).raw(), 1);
}

/// With `amount <= treasury_actual`, the ceiling burn of `revenue * amount / treasury_actual`
/// never exceeds `revenue`, so `burn_claimable_revenue` cannot burn more shares than the
/// treasury holds.
#[test]
fn mul_ratio_ceil_never_exceeds_the_base_when_ratio_is_at_most_one() {
    let env = Env::default();
    for revenue in [1i128, 2, 7, 1_000, 1_000_000, RAY, RAY * 1_000] {
        for (amount, actual) in [
            (1i128, 1i128),
            (1, 2),
            (1, 3),
            (2, 3),
            (999, 1_000),
            (1, 1_000_000),
            // amount == actual: the ratio is exactly one, the tightest case.
            (1_000_000, 1_000_000),
        ] {
            let burned = Ray::from(revenue)
                .mul_ratio_ceil(&env, amount, actual)
                .raw();
            assert!(
                burned <= revenue,
                "burned {burned} exceeds revenue {revenue} at ratio {amount}/{actual}"
            );
            assert!(
                burned > 0,
                "burned zero shares for a positive claim at ratio {amount}/{actual}"
            );
        }
    }
}

/// Every wrapper applies the rounding its name states at the scale its type
/// names. Operands are seeded random values of every magnitude; a result is
/// checked whenever the exact value is representable, against the inequalities
/// in `test_support` rather than against the primitive the wrapper calls.
#[test]
fn exact_wrappers_apply_the_named_rounding_at_the_named_scale() {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    let mut sampler = Sampler::new(6);

    for _ in 0..4_000 {
        // `a` is the receiver, `o` the other operand or divisor, `k` a second
        // divisor, `signed` a receiver for the operations that accept any sign.
        let (a, o, k) = (
            sampler.non_negative(),
            sampler.non_negative().max(1),
            sampler.non_negative().max(1),
        );
        let signed = sampler.signed();
        let decimals = sampler.below(46);
        let (ray, wad, bps) = (Ray::from(a), Wad::from(a), Bps::from(a));

        macro_rules! exact {
            ($fits:expr, $result:expr, |$q:ident| $holds:expr) => {
                if $fits {
                    let $q: i128 = $result;
                    assert!(
                        $holds,
                        "{} = {} (a = {a}, o = {o}, k = {k}, signed = {signed}, decimals = {decimals})",
                        stringify!($result),
                        $q
                    );
                }
            };
        }

        // Multiplication divides the product by the scale; division multiplies
        // the receiver by the scale and divides by the other operand.
        for (scale, product, scaled) in [
            (RAY, Wide::product(a, o), Wide::product(a, RAY)),
            (WAD, Wide::product(a, o), Wide::product(a, WAD)),
        ] {
            let is_ray = scale == RAY;
            exact!(
                half_up_fits(product, scale),
                if is_ray {
                    ray.mul(&env, Ray::from(o)).raw()
                } else {
                    wad.mul(&env, Wad::from(o)).raw()
                },
                |q| is_half_up(q, product, scale)
            );
            exact!(
                floor_fits(product, scale),
                if is_ray {
                    ray.mul_floor(&env, Ray::from(o)).raw()
                } else {
                    wad.mul_floor(&env, Wad::from(o)).raw()
                },
                |q| is_floor(q, product, scale)
            );
            exact!(
                ceil_fits(product, scale),
                if is_ray {
                    ray.mul_ceil(&env, Ray::from(o)).raw()
                } else {
                    wad.mul_ceil(&env, Wad::from(o)).raw()
                },
                |q| is_ceil(q, product, scale)
            );
            exact!(
                half_up_fits(scaled, o),
                if is_ray {
                    ray.div(&env, Ray::from(o)).raw()
                } else {
                    wad.div(&env, Wad::from(o)).raw()
                },
                |q| is_half_up(q, scaled, o)
            );
            exact!(
                floor_fits(scaled, o),
                if is_ray {
                    ray.div_floor(&env, Ray::from(o)).raw()
                } else {
                    wad.div_floor(&env, Wad::from(o)).raw()
                },
                |q| is_floor(q, scaled, o)
            );
        }

        let ray_product = Wide::product(a, o);
        let ray_scaled = Wide::product(a, RAY);
        exact!(
            ceil_fits(ray_scaled, o),
            ray.div_ceil(&env, Ray::from(o)).raw(),
            |q| is_ceil(q, ray_scaled, o)
        );
        exact!(
            half_up_fits(ray_scaled, o),
            Ray::from_fraction(&env, a, o).raw(),
            |q| is_half_up(q, ray_scaled, o)
        );
        exact!(
            ceil_fits(ray_product, k),
            ray.mul_ratio_ceil(&env, o, k).raw(),
            |q| is_ceil(q, ray_product, k)
        );
        exact!(true, Ray::from(signed).div_by_int(&env, o).raw(), |q| {
            is_half_away(q, Wide::from(signed), o)
        });

        // `try_mul` is `Some` exactly when the half-up product is representable,
        // and the saturating division clamps exactly when the floor is not.
        let wad_product = Wide::product(a, o);
        let tried = wad.try_mul(&env, Wad::from(o)).map(Wad::raw);
        assert_eq!(tried.is_some(), half_up_fits(wad_product, WAD), "{a} * {o}");
        exact!(tried.is_some(), tried.unwrap(), |q| is_half_up(
            q,
            wad_product,
            WAD
        ));
        let wad_scaled = Wide::product(a, WAD);
        exact!(
            true,
            wad.div_floor_saturating(&env, Wad::from(o)).raw(),
            |q| if floor_fits(wad_scaled, o) {
                is_floor(q, wad_scaled, o)
            } else {
                q == i128::MAX
            }
        );

        // Decimal conversions: exact when upscaling, the named rounding when
        // downscaling, for a receiver of either sign.
        let (ray_any, wad_any) = (Ray::from(signed), Wad::from(signed));
        exact!(true, ray_any.to_wad(&env).raw(), |q| {
            is_rescaled(q, signed, RAY_DECIMALS, WAD_DECIMALS, Rounding::HalfAway)
        });
        exact!(true, ray_any.to_wad_floor(&env).raw(), |q| {
            is_rescaled(q, signed, RAY_DECIMALS, WAD_DECIMALS, Rounding::TowardZero)
        });
        exact!(true, ray_any.to_wad_ceil(&env).raw(), |q| {
            is_rescaled(q, signed, RAY_DECIMALS, WAD_DECIMALS, Rounding::Ceil)
        });
        exact!(
            rescale_fits(signed, WAD_DECIMALS, RAY_DECIMALS),
            wad_any.to_ray(&env).raw(),
            |q| is_rescaled(q, signed, WAD_DECIMALS, RAY_DECIMALS, Rounding::HalfAway)
        );
        let from_ray = rescale_fits(signed, RAY_DECIMALS, decimals);
        exact!(from_ray, ray_any.to_asset(&env, decimals), |q| {
            is_rescaled(q, signed, RAY_DECIMALS, decimals, Rounding::HalfAway)
        });
        exact!(from_ray, ray_any.to_asset_floor(&env, decimals), |q| {
            is_rescaled(q, signed, RAY_DECIMALS, decimals, Rounding::TowardZero)
        });
        exact!(from_ray, ray_any.to_asset_ceil(&env, decimals), |q| {
            is_rescaled(q, signed, RAY_DECIMALS, decimals, Rounding::Ceil)
        });
        exact!(
            rescale_fits(signed, decimals, RAY_DECIMALS),
            Ray::from_asset(&env, signed, decimals).raw(),
            |q| is_rescaled(q, signed, decimals, RAY_DECIMALS, Rounding::HalfAway)
        );
        let from_wad = rescale_fits(signed, WAD_DECIMALS, decimals);
        exact!(from_wad, wad_any.to_token(&env, decimals), |q| {
            is_rescaled(q, signed, WAD_DECIMALS, decimals, Rounding::HalfAway)
        });
        exact!(from_wad, wad_any.to_token_floor(&env, decimals), |q| {
            is_rescaled(q, signed, WAD_DECIMALS, decimals, Rounding::TowardZero)
        });
        exact!(
            rescale_fits(signed, decimals, WAD_DECIMALS),
            Wad::from_token(&env, signed, decimals).raw(),
            |q| is_rescaled(q, signed, decimals, WAD_DECIMALS, Rounding::HalfAway)
        );

        // Basis points: the receiver is the rate, `o` the amount it applies to.
        let rated = Wide::product(o, a);
        let rated_fits = half_up_fits(rated, BPS);
        exact!(rated_fits, bps.apply_to(&env, o), |q| is_half_up(
            q, rated, BPS
        ));
        exact!(
            rated_fits,
            bps.apply_to_ray(&env, Ray::from(o)).raw(),
            |q| { is_half_up(q, rated, BPS) }
        );
        exact!(rated_fits, bps.flash_loan_fee_on(&env, o), |q| {
            if a > 0 && is_half_up(0, rated, BPS) {
                q == 1
            } else {
                is_half_up(q, rated, BPS)
            }
        });
        let ratio_exact = Wide::product(a, WAD);
        if half_up_fits(ratio_exact, BPS) {
            let ratio = bps.to_wad(&env).raw();
            assert!(
                is_half_up(ratio, ratio_exact, BPS),
                "Bps({a}).to_wad() = {ratio}"
            );
            let applied = Wide::product(o, ratio);
            exact!(
                half_up_fits(applied, WAD),
                bps.apply_to_wad(&env, Wad::from(o)).raw(),
                |q| is_half_up(q, applied, WAD)
            );
            exact!(
                floor_fits(applied, WAD),
                bps.apply_to_wad_floor(&env, Wad::from(o)).raw(),
                |q| is_floor(q, applied, WAD)
            );
        }
    }
}

/// The ratio, fraction and saturating wrappers each round once, in the
/// direction they are named for, on the exact product of their operands.
/// The `try_` forms are `None` exactly when that result is not representable.
#[test]
fn exact_ratio_and_fraction_wrappers_round_once_in_their_named_direction() {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    let mut sampler = Sampler::new(7);

    for _ in 0..4_000 {
        // `a` is the receiver, `o` and `k` the other operands or divisors,
        // `signed` a numerator of either sign.
        let (a, o, k) = (
            sampler.non_negative(),
            sampler.non_negative().max(1),
            sampler.non_negative().max(1),
        );
        let signed = sampler.signed();
        let decimals = sampler.below(46);
        let (ray, wad, bps) = (Ray::from(a), Wad::from(a), Bps::from(a));

        // Compares a panicking wrapper with the exact result where one exists.
        macro_rules! exact {
            ($expected:expr, $result:expr) => {
                if let Some(expected) = $expected {
                    assert_eq!(
                        $result,
                        expected,
                        "{} (a = {a}, o = {o}, k = {k}, decimals = {decimals})",
                        stringify!($result)
                    );
                }
            };
        }

        // Ratios multiply first and divide once by the denominator.
        exact!(
            half_up_of(a, o, k),
            ray.mul_ray_ratio(&env, Ray::from(o), Ray::from(k)).raw()
        );
        exact!(
            floor_of(a, o, k),
            ray.mul_wad_ratio_floor(&env, Wad::from(o), Wad::from(k))
                .raw()
        );
        exact!(floor_of(a, o, k), wad.mul_ratio_floor(&env, o, k).raw());
        exact!(
            floor_of(o, a, k),
            wad.apply_ratio_to_amount_floor(&env, Wad::from(k), o)
        );
        exact!(a.checked_mul(o), ray.mul_int(&env, o).raw());

        // Divisions scale the receiver up first; fractions scale the numerator.
        assert_eq!(
            ray.try_div(&env, Ray::from(o)).map(Ray::raw),
            half_up_of(a, RAY, o),
            "Ray({a}).try_div({o})"
        );
        // `Ray::try_mul` is `Some` exactly when the half-up product fits, and
        // then equals `Ray::mul`.
        assert_eq!(
            ray.try_mul(&env, Ray::from(o)).map(Ray::raw),
            half_up_of(a, o, RAY),
            "Ray({a}).try_mul({o})"
        );
        exact!(ceil_of(a, WAD, o), wad.div_ceil(&env, Wad::from(o)).raw());
        exact!(half_up_of(a, BPS, o), bps.div(&env, Bps::from(o)).raw());
        let (numerator, denominator) = (Wad::from(a), Wad::from(o));
        exact!(
            floor_of(a, BPS, o),
            Bps::from_fraction_floor(&env, numerator, denominator).raw()
        );
        exact!(
            ceil_of(a, BPS, o),
            Bps::from_fraction_ceil(&env, numerator, denominator).raw()
        );
        assert_eq!(
            Bps::try_from_fraction(&env, numerator, denominator).map(Bps::raw),
            half_up_of(a, BPS, o),
            "Bps::try_from_fraction({a}, {o})"
        );
        let weighted = Bps::from_weighted_sum(&env, signed, Wad::from(o)).raw();
        assert!(
            is_half_away(weighted, Wide::from(signed), o),
            "Bps::from_weighted_sum({signed}, {o}) = {weighted}"
        );

        // The saturating forms clamp to `i128::MAX` exactly when the floor
        // of a non-negative result does not fit.
        assert_eq!(
            ray.div_floor_saturating(&env, Ray::from(o)).raw(),
            floor_of(a, RAY, o).unwrap_or(i128::MAX),
            "Ray({a}).div_floor_saturating({o})"
        );
        assert_eq!(
            wad.mul_floor_saturating(&env, Wad::from(o)).raw(),
            floor_of(a, o, WAD).unwrap_or(i128::MAX),
            "Wad({a}).mul_floor_saturating({o})"
        );

        // Ceiling fees: the receiver is the rate, `o` the amount.
        exact!(ceil_of(o, a, BPS), bps.apply_to_ceil(&env, o));
        exact!(
            ceil_of(o, a, BPS),
            bps.apply_to_ray_ceil(&env, Ray::from(o)).raw()
        );

        // Token valuation divides by the token unit once; the upscale to WAD
        // is exact and rejects more than 18 decimals.
        assert_eq!(
            wad.try_value_of_token(&env, o, decimals).map(Wad::raw),
            10i128
                .checked_pow(decimals)
                .and_then(|unit| half_up_of(o, a, unit)),
            "Wad({a}).try_value_of_token({o}, {decimals})"
        );
        assert_eq!(
            Wad::try_from_token_upscale(signed, decimals).map(Wad::raw),
            WAD_DECIMALS
                .checked_sub(decimals)
                .filter(|_| signed >= 0)
                .and_then(|gap| signed.checked_mul(10i128.pow(gap))),
            "Wad::try_from_token_upscale({signed}, {decimals})"
        );
    }

    // Zero is a valid amount: only a negative one is rejected.
    assert_eq!(Wad::try_from_token_upscale(0, 7), Some(Wad::ZERO));
    assert_eq!(Wad::try_from_token_upscale(-1, 7), None);
}
