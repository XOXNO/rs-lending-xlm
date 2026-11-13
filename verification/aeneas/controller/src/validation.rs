//! `expect_invariant`, `require_nonneg_amount` and
//! `require_non_empty_payments`, copied verbatim from
//! `common/src/validation.rs` (the rest of that file needs the oracle module
//! and is not used by the included liquidation code).

use crate::errors::GenericError;
use soroban_sdk::{assert_with_error, panic_with_error, Env, Vec};

pub fn require_nonneg_amount(env: &Env, amount: i128) {
    assert_with_error!(env, amount >= 0, GenericError::AmountMustBePositive);
}

/// Returns the value contained in `opt`, panicking with
/// `GenericError::InternalError` if it is `None`.
#[inline]
pub fn expect_invariant<T>(env: &Env, opt: Option<T>) -> T {
    opt.unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError))
}

pub fn require_non_empty_payments<T>(env: &Env, payments: &Vec<T>) {
    assert_with_error!(env, !payments.is_empty(), GenericError::InvalidPayments);
}
