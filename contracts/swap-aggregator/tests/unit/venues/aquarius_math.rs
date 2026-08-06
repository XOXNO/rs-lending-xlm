use crate::residual_allowance;
use crate::venues::aquarius::{cp_swap_out, optimal_pre_swap, pre_balance_possible};
use soroban_sdk::Env;

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

#[test]
fn pre_balance_needs_something_to_balance_and_a_pool_to_balance_against() {
    assert!(pre_balance_possible(100, 100, 1_000, 1_000));

    assert!(pre_balance_possible(100, 0, 1_000, 1_000));
    assert!(pre_balance_possible(0, 100, 1_000, 1_000));

    assert!(!pre_balance_possible(0, 0, 1_000, 1_000));

    assert!(!pre_balance_possible(100, 100, 0, 1_000));
    assert!(!pre_balance_possible(100, 100, 1_000, 0));
}

#[test]
fn swap_output_follows_constant_product_with_the_fee_on_input() {
    assert_eq!(cp_swap_out(&Env::default(), 0, 1_000, 1_000, 30), 0);
    let out = cp_swap_out(&Env::default(), 1_000_000, 1_000_000_000, 1_000_000_000, 30);
    assert!(out > 995_000 && out < 1_000_000, "got {out}");

    assert_eq!(cp_swap_out(&Env::default(), 1_000, 1_000, 1_000, 10_000), 0);
}

#[test]
fn balanced_holdings_need_no_pre_swap() {
    let env = Env::default();

    let (_, amount) = optimal_pre_swap(&env, 1_000, 1_000, 5_000_000, 5_000_000, 30);
    assert_eq!(amount, 0);
    let (_, amount) = optimal_pre_swap(&env, 100, 200, 5_000_000, 10_000_000, 30);
    assert_eq!(amount, 0);
}

#[test]
fn pre_swap_sells_the_side_in_excess() {
    let env = Env::default();

    let (from_a, amount) = optimal_pre_swap(&env, 1_000_000, 0, 1_000_000_000, 1_000_000_000, 30);
    assert!(from_a, "the abundant side must be the one sold");
    assert!(
        amount > 490_000 && amount < 510_000,
        "expected about half, got {amount}"
    );

    let (from_a, amount) = optimal_pre_swap(&env, 0, 1_000_000, 1_000_000_000, 1_000_000_000, 30);
    assert!(!from_a);
    assert!(amount > 490_000 && amount < 510_000, "got {amount}");
}

#[test]
fn pre_swap_lands_on_the_pool_ratio() {
    let env = Env::default();
    let (reserve_a, reserve_b) = (1_000_000_000i128, 4_000_000_000i128);
    let held_a = 1_000_000i128;

    let (from_a, swap) = optimal_pre_swap(&env, held_a, 0, reserve_a, reserve_b, 30);
    assert!(from_a);
    let received = cp_swap_out(&env, swap, reserve_a, reserve_b, 30);

    let left = (held_a - swap) * (reserve_b - received);
    let right = received * (reserve_a + swap);
    let drift = (left - right).abs();
    assert!(
        drift <= right / 1_000,
        "holdings off the pool ratio by more than 0.1%: {left} vs {right}"
    );
}
