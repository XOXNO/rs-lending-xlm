use super::*;

#[test]
#[should_panic]
fn test_validate_and_calculate_tolerances_rejects_below_min() {
    let env = Env::default();
    let _ = validate_and_calculate_tolerances(&env, MIN_TOLERANCE - 1);
}

#[test]
#[should_panic]
fn test_validate_and_calculate_tolerances_rejects_above_max() {
    let env = Env::default();
    let _ = validate_and_calculate_tolerances(&env, MAX_TOLERANCE + 1);
}

#[test]
fn test_validate_and_calculate_tolerances_accepts_midpoint() {
    let env = Env::default();
    let tolerance = (MIN_TOLERANCE + MAX_TOLERANCE) / 2;
    let bands = validate_and_calculate_tolerances(&env, tolerance);
    assert!(bands.upper_ratio_bps > BPS as u32);
    assert!(bands.lower_ratio_bps < BPS as u32);
}

#[test]
fn test_calculate_tolerance_range_scales_bounds() {
    let env = Env::default();
    let (upper, lower) = calculate_tolerance_range(&env, 200);
    assert!(upper > BPS);
    assert!(lower < BPS);
}
