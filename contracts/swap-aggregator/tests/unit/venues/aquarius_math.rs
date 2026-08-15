use crate::residual_allowance;

#[test]
fn allowance_is_the_floor_until_proportional_overtakes_it() {
    assert_eq!(residual_allowance(0), 1_000);
    assert_eq!(residual_allowance(1_000_000), 1_000);

    assert_eq!(residual_allowance(1_000_000_000), 1_000);

    assert_eq!(residual_allowance(2_000_000_000), 2_000);
    assert_eq!(residual_allowance(10_000_000_000), 10_000);
}

// The accept/reject boundary of the residual guard is exercised through the
// real enforcement path in
// `execute_strategy::a_residual_of_exactly_the_allowance_passes_and_one_unit_more_reverts`;
// comparing integer literals against the allowance here proved nothing.

#[test]
fn small_trades_are_judged_against_the_floor_not_a_ratio() {
    assert_eq!(residual_allowance(5_000), 1_000);
    assert_eq!(residual_allowance(0), 1_000);
}
