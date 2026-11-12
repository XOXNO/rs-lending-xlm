use super::*;
use crate::constants::{MAX_BORROW_RATE_RAY, MILLISECONDS_PER_YEAR, RAY};
use crate::math::test_support::{half_up_of, Sampler};
use crate::rates::test_support::{assert_exact, unmetered_env};
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

/// Pins Taylor-term wiring against an unrolled expansion at sampled rates/times,
/// including stress rates; both paths share the Ray primitives.
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

#[test]
#[should_panic(expected = "Error(Contract, #33)")]
fn test_compound_interest_scaled_exponent_overflow_is_math_overflow() {
    let env = Env::default();
    let _ = compound_interest(&env, Ray::from(i128::MAX), 2);
}

/// The documented series, computed exactly: `1 + x + sum(x^k / k!)` for `k`
/// in `2..=8` with `x = rate * delta_ms`, every power and every term rounded
/// half up. `None` when a step does not fit `i128`.
fn exact_compound(rate: i128, delta_ms: u64) -> Option<i128> {
    if delta_ms == 0 {
        return Some(RAY);
    }
    let x = rate.checked_mul(i128::from(delta_ms))?;
    let mut sum = RAY.checked_add(x)?;
    let (mut power, mut factorial) = (x, 1);
    for k in 2..=8 {
        factorial *= k;
        power = half_up_of(power, x, RAY)?;
        sum = sum.checked_add(half_up_of(power, 1, factorial)?)?;
    }
    Some(sum)
}

#[test]
fn exact_compound_interest_matches_the_rounded_taylor_series() {
    let env = unmetered_env();
    let check = |rate: i128, delta_ms: u64| {
        assert_exact(
            &env,
            exact_compound(rate, delta_ms),
            |env| compound_interest(env, Ray::from(rate), delta_ms).raw(),
            (rate, delta_ms),
        );
    };
    let year = i128::from(MILLISECONDS_PER_YEAR);

    check(0, 1);
    check(0, u64::MAX);
    check(1, 1);
    // The largest exponent the protocol reaches: the rate cap over one full chunk.
    let capped_rate = half_up_of(MAX_BORROW_RATE_RAY, 1, year).unwrap();
    check(capped_rate, MAX_COMPOUND_DELTA_MS);

    let mut sampler = Sampler::new(21);
    for _ in 0..4_000 {
        // The protocol domain: a per-millisecond rate under the cap, over at
        // most one chunk.
        let rate = half_up_of(sampler.at_most(MAX_BORROW_RATE_RAY), 1, year).unwrap();
        check(rate, sampler.at_most(year) as u64);
    }
    for _ in 0..1_000 {
        // Exponents up to 64: the eighth power stops fitting `i128` near 25.
        check(sampler.at_most(64 * RAY), 1);
        // Any rate over any interval is exact or a typed overflow.
        check(
            sampler.non_negative(),
            sampler.at_most(i128::from(u64::MAX)) as u64,
        );
    }
}

/// One year at each rate the doc comment quotes. `exp` is `e^x * RAY` for the
/// exponent the function uses, `x = half_up(annual / year) * year`, computed
/// to 80 digits outside the crate. The relative shortfall, rounded to three
/// significant digits, is the documented figure.
#[test]
fn compound_interest_shortfall_against_exp_matches_the_documented_figures() {
    let env = Env::default();
    // (annual rate, exp, shortfall digits, negated power of ten)
    let cases: [(i128, i128, i128, u32); 4] = [
        (2 * RAY, 7_389_056_098_930_650_222_963_158_895, 237, 6),
        (RAY, 2_718_281_828_459_045_234_575_367_284, 113, 8),
        (RAY / 5, 1_221_402_758_160_169_841_559_277_811, 118, 14),
        (RAY / 20, 1_051_271_096_376_024_049_634_804_853, 515, 20),
    ];
    for (annual, exp, digits, power) in cases {
        let rate = Ray::from(annual).div_by_int(&env, i128::from(MILLISECONDS_PER_YEAR));
        let factor = compound_interest(&env, rate, MILLISECONDS_PER_YEAR).raw();
        let shortfall = exp - factor;
        assert!(
            shortfall > 0,
            "annual rate {annual}: truncation underestimates"
        );
        assert_eq!(
            (shortfall * 10i128.pow(power) + exp / 2) / exp,
            digits,
            "annual rate {annual}: relative shortfall in units of 1e-{power}"
        );
    }
}
