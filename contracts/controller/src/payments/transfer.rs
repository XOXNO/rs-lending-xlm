//! Controller-local wrappers over `common::token` transfer helpers.

use common::errors::GenericError;
use common::token::transfer_amount_measured as transfer_amount_measured_common;
use soroban_sdk::{panic_with_error, token, Address, Env};

/// Transfers `amount` of `asset` from `from` to `to` and returns the amount actually
/// credited to `to`, measured as the recipient's balance delta before and after the
/// transfer. Panics with `non_positive_error` if `amount` is not positive.
pub(crate) fn transfer_amount_measured(
    env: &Env,
    asset: &Address,
    from: &Address,
    to: &Address,
    amount: i128,
    non_positive_error: GenericError,
) -> i128 {
    transfer_amount_measured_common(env, asset, from, to, amount, non_positive_error)
}

/// Returns the current contract's balance of `token` minus `balance_before`.
/// Panics with `GenericError::InternalError` if the subtraction overflows `i128`.
pub(crate) fn balance_delta(env: &Env, token: &token::Client, balance_before: i128) -> i128 {
    token
        .balance(&env.current_contract_address())
        .checked_sub(balance_before)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError))
}
