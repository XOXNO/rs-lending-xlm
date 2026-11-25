use crate::math::scale_amount;
use common::math::RatioRounding;
use soroban_sdk::Env;

#[test]
fn scale_amount_rounds_once_and_accepts_zero_operands() {
    let env = Env::default();
    assert_eq!(scale_amount(&env, 10, 3, 2, RatioRounding::Floor), 15);
    assert_eq!(scale_amount(&env, 10, 1, 3, RatioRounding::Floor), 3);
    assert_eq!(scale_amount(&env, 10, 1, 3, RatioRounding::Ceil), 4);
    assert_eq!(scale_amount(&env, 0, 3, 2, RatioRounding::Ceil), 0);
    assert_eq!(scale_amount(&env, 5, 0, 3, RatioRounding::Ceil), 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn scale_amount_rejects_a_negative_amount() {
    scale_amount(&Env::default(), -1, 1, 1, RatioRounding::Floor);
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn scale_amount_rejects_a_negative_numerator() {
    scale_amount(&Env::default(), 1, -1, 1, RatioRounding::Floor);
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn scale_amount_rejects_a_zero_denominator() {
    scale_amount(&Env::default(), 1, 1, 0, RatioRounding::Floor);
}
