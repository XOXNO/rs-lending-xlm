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

/// Nonnegative token quantities; widen products rather than narrowing ratios.
pub(crate) fn mul_div_floor(env: &Env, a: i128, b: i128, d: i128) -> i128 {
    mul_div(env, a, b, d, false)
}
pub(crate) fn mul_div_ceil(env: &Env, a: i128, b: i128, d: i128) -> i128 {
    mul_div(env, a, b, d, true)
}
fn mul_div(env: &Env, a: i128, b: i128, d: i128, ceil: bool) -> i128 {
    use soroban_sdk::U256;
    if a < 0 || b < 0 || d <= 0 {
        panic_with_error!(env, Error::InvalidAmount);
    }
    if let Some(product) = a.checked_mul(b) {
        return product / d + i128::from(ceil && product % d != 0);
    }
    let wide = |v: i128| U256::from_u128(env, v as u128);
    let product = wide(a).mul(&wide(b));
    let numerator = if ceil {
        product.add(&wide(d - 1))
    } else {
        product
    };
    numerator
        .div(&wide(d))
        .to_u128()
        .and_then(|x| i128::try_from(x).ok())
        .unwrap_or_else(|| panic_with_error!(env, Error::IntegerOverflow))
}
