use super::*;

#[test]
#[should_panic]
fn test_validate_and_calculate_tolerances_rejects_last_lte_first() {
    let env = Env::default();
    let _ = validate_and_calculate_tolerances(&env, MIN_FIRST_TOLERANCE, MIN_FIRST_TOLERANCE);
}

#[test]
#[should_panic]
fn test_validate_and_calculate_tolerances_rejects_first_below_min() {
    let env = Env::default();
    let _ = validate_and_calculate_tolerances(&env, MIN_FIRST_TOLERANCE - 1, MIN_LAST_TOLERANCE);
}

#[test]
fn test_calculate_tolerance_range_scales_bounds() {
    let env = Env::default();
    let (upper, lower) = calculate_tolerance_range(&env, 200);
    assert!(upper > BPS);
    assert!(lower < BPS);
}
