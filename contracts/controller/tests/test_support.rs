use crate::storage;
use soroban_sdk::Env;

pub fn set_flash_loan_ongoing(env: &Env, ongoing: bool) {
    storage::set_flash_loan_ongoing(env, ongoing);
}

#[must_use]
pub fn is_flash_loan_ongoing(env: &Env) -> bool {
    storage::is_flash_loan_ongoing(env)
}
