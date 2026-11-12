use super::*;
use crate::constants::{BPS, MAX_BORROW_RATE_RAY, MILLISECONDS_PER_YEAR, RAY};
use crate::math::reference::div_by_int_half_up;
use crate::math::test_support::{half_up_of, Sampler};
use crate::rates::test_support::*;
use crate::types::MarketParamsRaw;
use soroban_sdk::Env;

#[test]
fn test_borrow_rate_region1() {
    let env = Env::default();
    let params = make_test_params(&env);

    let rate = calculate_borrow_rate(&env, Ray::ZERO, &params);
    let expected = div_by_int_half_up(&env, RAY / 100, MILLISECONDS_PER_YEAR as i128);
    assert_eq!(rate.raw(), expected);

    let util_25 = Ray::from(RAY * 25 / 100);
    let rate = calculate_borrow_rate(&env, util_25, &params);
    let expected_annual = RAY * 3 / 100;
    let expected_per_ms = div_by_int_half_up(&env, expected_annual, MILLISECONDS_PER_YEAR as i128);
    assert!((rate.raw() - expected_per_ms).abs() <= 1);
}

#[test]
fn test_borrow_rate_region2() {
    let env = Env::default();
    let params = make_test_params(&env);

    let util_50 = Ray::from(RAY * 50 / 100);
    let rate = calculate_borrow_rate(&env, util_50, &params);
    let expected_annual = RAY * 5 / 100;
    let expected_per_ms = div_by_int_half_up(&env, expected_annual, MILLISECONDS_PER_YEAR as i128);
    assert!((rate.raw() - expected_per_ms).abs() <= 1);

    let util_65 = Ray::from(RAY * 65 / 100);
    let rate = calculate_borrow_rate(&env, util_65, &params);
    let expected_annual = RAY * 10 / 100;
    let expected_per_ms = div_by_int_half_up(&env, expected_annual, MILLISECONDS_PER_YEAR as i128);
    assert!((rate.raw() - expected_per_ms).abs() <= 1);
}

#[test]
fn test_borrow_rate_region3() {
    let env = Env::default();
    let params = make_test_params(&env);

    let util_80 = Ray::from(RAY * 80 / 100);
    let rate = calculate_borrow_rate(&env, util_80, &params);
    let expected_annual = RAY * 15 / 100;
    let expected_per_ms = div_by_int_half_up(&env, expected_annual, MILLISECONDS_PER_YEAR as i128);
    assert!((rate.raw() - expected_per_ms).abs() <= 1);

    let util_90 = Ray::from(RAY * 90 / 100);
    let rate = calculate_borrow_rate(&env, util_90, &params);
    let expected_annual = RAY;
    let expected_per_ms = div_by_int_half_up(&env, expected_annual, MILLISECONDS_PER_YEAR as i128);
    assert!((rate.raw() - expected_per_ms).abs() <= 1);
}

#[test]
fn test_borrow_rate_capped() {
    let env = Env::default();
    let params = make_test_params(&env);

    let rate = calculate_borrow_rate(&env, Ray::ONE, &params);
    let expected = div_by_int_half_up(
        &env,
        params.max_borrow_rate.raw(),
        MILLISECONDS_PER_YEAR as i128,
    );
    assert!((rate.raw() - expected).abs() <= 1);
}

#[test]
fn test_borrow_rate_clamps_utilization_above_one() {
    let env = Env::default();

    let mut raw = make_test_params_raw(&env);
    raw.optimal_utilization = RAY - 1;
    raw.max_utilization = RAY;
    let params = MarketParams::from(&raw);

    let rate = calculate_borrow_rate(&env, Ray::from(RAY * 2), &params);
    let expected = div_by_int_half_up(
        &env,
        params.max_borrow_rate.raw(),
        MILLISECONDS_PER_YEAR as i128,
    );
    assert!(rate.raw() > 0);
    assert!(
        rate.raw() <= expected + 1,
        "util > RAY must clamp and stay bounded by the max-rate cap"
    );
}

#[test]
fn test_calculate_borrow_rate_mid_utilization_boundary_exact() {
    let env = Env::default();
    let mut params = make_test_params(&env);
    params.mid_utilization = Ray::from(RAY / 3);
    params.slope1 = Ray::from(186_742_236_914_318_803_376_138_999_i128);
    params.optimal_utilization = params.mid_utilization.checked_add(&env, Ray::from(RAY / 5));

    let rate = calculate_borrow_rate(&env, params.mid_utilization, &params);

    assert_eq!(
        rate.raw(),
        6_234_518_435_487_626_i128,
        "utilization == mid_utilization must take the slope2 branch (zero contribution)"
    );
}

#[test]
fn test_calculate_borrow_rate_optimal_utilization_boundary_exact() {
    let env = Env::default();
    let mut params = make_test_params(&env);
    params.mid_utilization = Ray::from(RAY / 5);
    params.slope1 = Ray::ZERO;
    params.slope2 = Ray::from(186_742_236_914_318_803_376_138_999_i128);
    params.optimal_utilization = params.mid_utilization.checked_add(&env, Ray::from(RAY / 3));

    let rate = calculate_borrow_rate(&env, params.optimal_utilization, &params);

    assert_eq!(
        rate.raw(),
        6_234_518_435_487_626_i128,
        "utilization == optimal_utilization must take the slope3 branch (zero contribution)"
    );
}

#[test]
fn test_annual_borrow_rate_matches_curve_before_year_conversion() {
    let env = Env::default();
    let params = make_test_params(&env);

    assert_eq!(
        calculate_annual_borrow_rate(&env, Ray::ZERO, &params).raw(),
        RAY / 100
    );

    let util_50 = Ray::from(RAY * 50 / 100);
    assert_eq!(
        calculate_annual_borrow_rate(&env, util_50, &params).raw(),
        RAY * 5 / 100
    );
}

#[test]
fn test_borrow_rate_is_annual_divided_by_milliseconds_per_year() {
    let env = Env::default();
    let params = make_test_params(&env);
    let util = Ray::from(RAY * 65 / 100);
    let annual = calculate_annual_borrow_rate(&env, util, &params);
    let per_ms = calculate_borrow_rate(&env, util, &params);
    assert_eq!(
        per_ms.raw(),
        annual.div_by_int(&env, MILLISECONDS_PER_YEAR as i128).raw()
    );
}

#[test]
fn test_deposit_rate() {
    let env = Env::default();
    let util_80 = Ray::from(RAY * 80 / 100);
    let borrow_rate = Ray::from(RAY * 5 / 100);
    let reserve_factor = Bps::from(1000);

    let rate = calculate_deposit_rate(&env, util_80, borrow_rate, reserve_factor);

    let expected = RAY * 36 / 1000;
    assert!(
        (rate.raw() - expected).abs() <= 1,
        "rate={}, expected={}",
        rate.raw(),
        expected
    );
}

#[test]
fn test_deposit_rate_zero_util() {
    let env = Env::default();
    assert_eq!(
        calculate_deposit_rate(&env, Ray::ZERO, Ray::from(RAY / 10), Bps::from(1000)),
        Ray::ZERO
    );
}

#[test]
fn test_deposit_rate_reserve_factor_out_of_range_returns_zero() {
    let env = Env::default();

    let rate = calculate_deposit_rate(
        &env,
        Ray::from(RAY / 2),
        Ray::from(RAY / 10),
        Bps::from(crate::constants::BPS),
    );
    assert_eq!(rate, Ray::ZERO);
}

#[test]
fn test_utilization_basic() {
    let env = Env::default();
    let util = utilization(&env, Ray::from(50 * RAY), Ray::from(100 * RAY));
    let expected = RAY / 2;
    assert!((util.raw() - expected).abs() <= 1);
}

#[test]
fn test_utilization_zero_supplied() {
    let env = Env::default();
    assert_eq!(utilization(&env, Ray::from(50 * RAY), Ray::ZERO), Ray::ZERO);
}

#[test]
fn test_annual_borrow_rate_region3_uncapped_with_valid_params() {
    let env = Env::default();
    let mut raw = make_test_params_raw(&env);
    raw.slope3 = 500_000_000_000_000_000_000_000_000;
    raw.verify(&env);
    let params = MarketParams::from(&raw);
    let utilization = Ray::from(900_000_000_000_000_000_000_000_000_i128);
    assert_eq!(
        calculate_annual_borrow_rate(&env, utilization, &params).raw(),
        400_000_000_000_000_000_000_000_000
    );
}

#[test]
fn test_utilization_saturates_when_ratio_overflows() {
    let env = Env::default();
    let util = utilization(
        &env,
        Ray::from(89_945_816_546_977_511_412i128),
        Ray::from(7_674i128),
    );
    assert_eq!(util, Ray::ONE);
}

#[test]
fn test_utilization_keeps_largest_representable_ratio() {
    let env = Env::default();
    let ratio = 170_141_183_460 * RAY;
    assert_eq!(
        utilization(&env, Ray::from(ratio), Ray::ONE),
        Ray::from(ratio)
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #33)")]
fn test_utilization_rejects_negative_borrowed() {
    let env = Env::default();
    let _ = utilization(&env, Ray::from(-1i128), Ray::ONE);
}

#[test]
#[should_panic(expected = "Error(Contract, #33)")]
fn test_utilization_rejects_negative_supplied() {
    let env = Env::default();
    let _ = utilization(&env, Ray::ONE, Ray::from(-1i128));
}

/// The capped annual rate as documented: each segment adds
/// `slope * excess / width`, rounded half up once.
fn exact_curve_rate(utilization: i128, params: &MarketParams) -> Option<i128> {
    let utilization = utilization.min(RAY);
    let (mid, optimal) = (
        params.mid_utilization.raw(),
        params.optimal_utilization.raw(),
    );
    let (base, slope1, slope2, slope3) = (
        params.base_borrow_rate.raw(),
        params.slope1.raw(),
        params.slope2.raw(),
        params.slope3.raw(),
    );
    let rate = if utilization < mid {
        base + half_up_of(utilization, slope1, mid)?
    } else if utilization < optimal {
        base + slope1 + half_up_of(utilization - mid, slope2, optimal - mid)?
    } else {
        base + slope1 + slope2 + half_up_of(utilization - optimal, slope3, RAY - optimal)?
    };
    Some(rate.min(params.max_borrow_rate.raw()))
}

/// A top segment one unit wide adds exactly its slope at full utilization.
/// Rounding the product before the divide would scale that rounding error by
/// `RAY / width` and turn this 60% slope into 100%.
#[test]
fn borrow_rate_on_a_one_unit_segment_adds_exactly_its_slope() {
    let env = Env::default();
    let params = MarketParams::from(&MarketParamsRaw {
        base_borrow_rate: 0,
        slope1: 0,
        slope2: 0,
        slope3: RAY * 6 / 10,
        max_borrow_rate: MAX_BORROW_RATE_RAY,
        mid_utilization: RAY / 2,
        optimal_utilization: RAY - 1,
        max_utilization: RAY,
        ..make_test_params_raw(&env)
    });

    let below = calculate_annual_borrow_rate(&env, Ray::from(RAY - 1), &params);
    let full = calculate_annual_borrow_rate(&env, Ray::ONE, &params);
    assert_eq!(below, Ray::ZERO);
    assert_eq!(full, params.slope3);
}

#[test]
fn exact_borrow_rate_follows_the_piecewise_curve() {
    let env = unmetered_env();
    let year = i128::from(MILLISECONDS_PER_YEAR);
    let mut sampler = Sampler::new(31);
    for _ in 0..2_000 {
        let mut params = sample_params(&env, &mut sampler);
        // Listing validation keeps the cap above the curve. Half the cases
        // lower it into the curve, which the function must still honour.
        if sampler.below(2) == 0 {
            params.max_borrow_rate = Ray::from(sampler.at_most(params.max_borrow_rate.raw()));
        }
        let (mid, optimal) = (
            params.mid_utilization.raw(),
            params.optimal_utilization.raw(),
        );
        let annual_at = |utilization: i128| {
            calculate_annual_borrow_rate(&env, Ray::from(utilization), &params).raw()
        };

        let mut utilizations = [
            0,
            sampler.at_most(mid),
            mid - 1,
            mid,
            mid + sampler.at_most(optimal - mid),
            optimal - 1,
            optimal,
            optimal + sampler.at_most(RAY - optimal),
            RAY - 1,
            RAY,
            RAY + 1 + sampler.at_most(RAY),
        ];
        utilizations.sort_unstable();
        let mut previous = 0;
        for utilization in utilizations {
            let annual = annual_at(utilization);
            assert_eq!(
                Some(annual),
                exact_curve_rate(utilization, &params),
                "utilization {utilization} under {params:?}"
            );
            // The rate never falls as utilization rises, inside a segment or
            // across a kink.
            assert!(
                annual >= previous,
                "utilization {utilization} under {params:?}: {annual} after {previous}"
            );
            previous = annual;

            assert_eq!(
                Some(calculate_borrow_rate(&env, Ray::from(utilization), &params).raw()),
                half_up_of(annual, 1, year),
                "utilization {utilization} under {params:?}"
            );
        }

        // Each kink sits exactly on the sum of the slopes below it, up to the cap.
        let cap = params.max_borrow_rate.raw();
        let after_slope1 = params.base_borrow_rate.raw() + params.slope1.raw();
        let after_slope2 = after_slope1 + params.slope2.raw();
        let after_slope3 = after_slope2 + params.slope3.raw();
        assert_eq!(annual_at(mid), after_slope1.min(cap), "{params:?}");
        assert_eq!(annual_at(optimal), after_slope2.min(cap), "{params:?}");
        assert_eq!(annual_at(RAY), after_slope3.min(cap), "{params:?}");
    }
}

#[test]
fn exact_utilization_and_deposit_rate_round_half_up() {
    let env = unmetered_env();
    let mut sampler = Sampler::new(32);
    for _ in 0..5_000 {
        // Any non-negative pair: a ratio beyond `i128` reads as full utilization.
        let (borrowed, supplied) = (sampler.non_negative(), sampler.non_negative());
        let expected = if supplied == 0 {
            0
        } else {
            half_up_of(borrowed, RAY, supplied).unwrap_or(RAY)
        };
        assert_eq!(
            utilization(&env, Ray::from(borrowed), Ray::from(supplied)).raw(),
            expected,
            "{borrowed} borrowed against {supplied} supplied"
        );

        let (used, rate) = (
            sampler.at_most(2 * RAY),
            sampler.at_most(MAX_BORROW_RATE_RAY),
        );
        // From one below to one above the accepted `0..BPS` range.
        let reserve_factor = sampler.at_most(BPS + 1) - 1;
        let expected = if used == 0 || !(0..BPS).contains(&reserve_factor) {
            Some(0)
        } else {
            half_up_of(used, rate, RAY)
                .and_then(|rate_x_util| half_up_of(rate_x_util, BPS - reserve_factor, BPS))
        };
        let deposit_rate = calculate_deposit_rate(
            &env,
            Ray::from(used),
            Ray::from(rate),
            Bps::from(reserve_factor),
        );
        assert_eq!(
            Some(deposit_rate.raw()),
            expected,
            "utilization {used}, rate {rate}, reserve factor {reserve_factor}"
        );
    }
}
