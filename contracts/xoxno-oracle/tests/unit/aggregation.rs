extern crate std;

use super::*;
use soroban_sdk::{vec, Env};

#[test]
fn sorted_copy_orders_ascending_and_leaves_the_input_untouched() {
    let env = Env::default();
    let input = vec![&env, 5i128, 1, 4, 2, 3];

    let sorted = sorted_copy(&input);

    assert_eq!(sorted, vec![&env, 1i128, 2, 3, 4, 5]);
    assert_eq!(
        input,
        vec![&env, 5i128, 1, 4, 2, 3],
        "input must not be mutated"
    );
}

#[test]
fn sorted_copy_handles_duplicates_and_negatives() {
    let env = Env::default();
    let sorted = sorted_copy(&vec![&env, 3i128, -1, 3, 0, -1]);
    assert_eq!(sorted, vec![&env, -1i128, -1, 0, 3, 3]);
}

#[test]
fn sorted_copy_is_a_noop_for_zero_and_one_element() {
    let env = Env::default();
    assert_eq!(sorted_copy(&vec![&env]), vec![&env]);
    assert_eq!(sorted_copy(&vec![&env, 7i128]), vec![&env, 7i128]);
}

#[test]
fn median_of_odd_count_is_the_middle_element() {
    let env = Env::default();
    assert_eq!(median_of(&vec![&env, 30i128, 10, 20]), 20);
}

#[test]
fn median_of_even_count_takes_the_lower_middle_not_the_average() {
    let env = Env::default();
    assert_eq!(median_of(&vec![&env, 10i128, 20, 30, 1_000]), 20);
}

#[test]
fn median_of_single_element_is_that_element() {
    let env = Env::default();
    assert_eq!(median_of(&vec![&env, 42i128]), 42);
}

#[test]
fn median_of_ignores_a_single_extreme_outlier() {
    let env = Env::default();
    let honest = vec![&env, 100i128, 101, 99, 100, i128::MAX];
    assert_eq!(median_of(&honest), 100);
}
