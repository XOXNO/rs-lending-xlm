//! Overflow-checked arithmetic; panics with [`Error::IntegerOverflow`].

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
