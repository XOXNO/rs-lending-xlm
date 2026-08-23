//! First-pass killers for SAC helpers. A no-op `()` mutant must not survive a
//! positive-amount transfer against a missing token contract.
extern crate std;

use super::*;
use crate::errors::GenericError;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

#[test]
#[should_panic(expected = "Error(Contract, #14)")]
fn transfer_amount_measured_rejects_zero() {
    let env = Env::default();
    let _ = transfer_amount_measured(
        &env,
        &Address::generate(&env),
        &Address::generate(&env),
        &Address::generate(&env),
        0,
        GenericError::AmountMustBePositive,
    );
}

#[test]
#[should_panic]
fn transfer_amount_measured_positive_requires_token() {
    let env = Env::default();
    let _ = transfer_amount_measured(
        &env,
        &Address::generate(&env),
        &Address::generate(&env),
        &Address::generate(&env),
        1,
        GenericError::AmountMustBePositive,
    );
}
