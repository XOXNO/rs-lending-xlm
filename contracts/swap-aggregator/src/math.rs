//! Overflow-checked arithmetic; panics with [`Error::IntegerOverflow`].

use common::math::fp::RatioRounding;
use soroban_sdk::{panic_with_error, Env};

use crate::errors::Error;

/// Adds `lhs` and `rhs`, panicking on overflow.
pub(crate) fn checked_add(env: &Env, lhs: i128, rhs: i128) -> i128 {
    lhs.checked_add(rhs)
        .unwrap_or_else(|| panic_with_error!(env, Error::IntegerOverflow))
}

/// Multiplies `lhs` and `rhs`, panicking on overflow.
pub(crate) fn checked_mul(env: &Env, lhs: i128, rhs: i128) -> i128 {
    lhs.checked_mul(rhs)
        .unwrap_or_else(|| panic_with_error!(env, Error::IntegerOverflow))
}

/// Scales a nonnegative token amount while preserving router error codes.
pub(crate) fn scale_amount(
    env: &Env,
    amount: i128,
    numerator: i128,
    denominator: i128,
    rounding: RatioRounding,
) -> i128 {
    if amount < 0 || numerator < 0 || denominator <= 0 {
        panic_with_error!(env, Error::InvalidAmount);
    }
    rounding
        .checked_scale(env, amount, numerator, denominator)
        .unwrap_or_else(|| panic_with_error!(env, Error::IntegerOverflow))
}
