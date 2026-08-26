use super::*;
use crate::constants::{MILLISECONDS_PER_YEAR, RAY};
use soroban_sdk::Env;

#[test]
fn test_compound_interest_zero_delta() {
    let env = Env::default();
    let result = compound_interest(&env, Ray::from(RAY / 10), 0);
    assert_eq!(result, Ray::ONE);
}

#[test]
fn test_compound_interest_accuracy() {
    let env = Env::default();

    let annual_rate = Ray::from(RAY / 10);
    let rate_per_ms = annual_rate.div_by_int(&env, MILLISECONDS_PER_YEAR as i128);
    let factor = compound_interest(&env, rate_per_ms, MILLISECONDS_PER_YEAR);

    let expected_e_010 = 1_105_170_918_075_647_624_811_707_826_i128;

    let diff = (factor.raw() - expected_e_010).abs();
    let tolerance = expected_e_010 / 1_000_000;
    assert!(
        diff < tolerance,
        "Compound interest accuracy: factor={}, expected={}, diff={}, tolerance={}",
        factor.raw(),
        expected_e_010,
        diff,
        tolerance
    );
}

#[test]
fn test_compound_interest_high_x_pins_all_taylor_terms() {
    let env = Env::default();

    let rate = Ray::from(RAY / 2);
    let result = compound_interest(&env, rate, 1);

    let expected = 1_648_721_270_700_128_146_848_650_787_i128;

    let tolerance = 1e19 as i128;
    let diff = (result.raw() - expected).abs();
    assert!(
        diff <= tolerance,
        "compound_interest(0.5) drift {} exceeds tolerance {}; got {}, expected {}",
        diff,
        tolerance,
        result.raw(),
        expected
    );
}

/// Pins the Taylor-term loop against the hand-unrolled `x^2..x^8` form it replaced: same
/// operation order, same rounding, bit-identical results across the supported rate and
/// elapsed-time domain.
#[test]
fn test_compound_interest_matches_the_unrolled_taylor_expansion() {
    fn unrolled(env: &Env, x: Ray) -> Ray {
        let x_sq = x.mul(env, x);
        let x_cub = x_sq.mul(env, x);
        let x_pow4 = x_cub.mul(env, x);
        let x_pow5 = x_pow4.mul(env, x);
        let x_pow6 = x_pow5.mul(env, x);
        let x_pow7 = x_pow6.mul(env, x);
        let x_pow8 = x_pow7.mul(env, x);

        let terms = [
            x,
            x_sq.div_by_int(env, 2),
            x_cub.div_by_int(env, 6),
            x_pow4.div_by_int(env, 24),
            x_pow5.div_by_int(env, 120),
            x_pow6.div_by_int(env, 720),
            x_pow7.div_by_int(env, 5_040),
            x_pow8.div_by_int(env, 40_320),
        ];
        let mut sum = Ray::ONE;
        for term in terms {
            sum = sum.checked_add(env, term);
        }
        sum
    }

    let env = Env::default();
    for rate_bps in [0i128, 1, 7, 250, 1_000, 5_000, 10_000, 100_000] {
        let rate = Ray::from(RAY * rate_bps / 10_000 / MILLISECONDS_PER_YEAR as i128);
        for delta_ms in [
            1u64,
            1_000,
            60_000,
            86_400_000,
            MILLISECONDS_PER_YEAR / 12,
            MILLISECONDS_PER_YEAR,
        ] {
            let x = Ray::from(rate.raw() * delta_ms as i128);
            assert_eq!(
                compound_interest(&env, rate, delta_ms),
                unrolled(&env, x),
                "rate_bps={rate_bps} delta_ms={delta_ms}"
            );
        }
    }
}
