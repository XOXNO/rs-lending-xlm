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
    let rate_per_ms = annual_rate.div_by_int(MILLISECONDS_PER_YEAR as i128);
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
