use super::*;

#[test]
#[should_panic(expected = "Error(Contract, #208)")]
fn validate_and_calculate_tolerances_rejects_below_min() {
    let env = Env::default();
    let _ = validate_and_calculate_tolerances(&env, MIN_TOLERANCE - 1);
}

#[test]
#[should_panic(expected = "Error(Contract, #208)")]
fn validate_and_calculate_tolerances_rejects_above_max() {
    let env = Env::default();
    let _ = validate_and_calculate_tolerances(&env, MAX_TOLERANCE + 1);
}

#[test]
fn validate_and_calculate_tolerances_returns_expected_band() {
    let env = Env::default();
    let tolerance = (MIN_TOLERANCE + MAX_TOLERANCE) / 2;
    let bands = validate_and_calculate_tolerances(&env, tolerance);
    assert_eq!(bands.upper_ratio_bps, 11_325);
    assert_eq!(bands.lower_ratio_bps, 8_830);
}

#[test]
fn calculate_tolerance_range_scales_bounds() {
    let env = Env::default();
    let (upper, lower) = calculate_tolerance_range(&env, 200);
    assert_eq!(upper, 10_200);
    assert_eq!(lower, 9_804);
}
