use crate::residual_allowance;

#[test]
fn allowance_is_the_floor_until_proportional_overtakes_it() {
    assert_eq!(residual_allowance(0), 1_000);
    assert_eq!(residual_allowance(1_000_000), 1_000);

    assert_eq!(residual_allowance(1_000_000_000), 1_000);

    assert_eq!(residual_allowance(2_000_000_000), 2_000);
    assert_eq!(residual_allowance(10_000_000_000), 10_000);
}

#[test]
fn residual_is_allowed_up_to_and_including_the_allowance() {
    let credited = 10_000_000_000i128;
    let allowance = residual_allowance(credited);
    assert_eq!(allowance, 10_000);
    assert!(9_999 <= allowance, "just under must be allowed");
    assert!(10_000 <= allowance, "exactly the allowance must be allowed");
    assert!(10_001 > allowance, "one past it must not be");
}

#[test]
fn small_trades_are_judged_against_the_floor_not_a_ratio() {
    assert_eq!(residual_allowance(5_000), 1_000);
    assert_eq!(residual_allowance(0), 1_000);
}
