use common::errors::GenericError;
use common::token::transfer_amount_measured as transfer_amount_measured_common;
use soroban_sdk::{panic_with_error, token, Address, Env};

/// Transfer and credit only what actually arrived.
///
/// Thin wrapper over [`common::token::transfer_amount_measured`] so call sites
/// keep the controller-local import path documented in ADR 0013.
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

pub(crate) fn balance_delta(env: &Env, token: &token::Client, balance_before: i128) -> i128 {
    token
        .balance(&env.current_contract_address())
        .checked_sub(balance_before)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError))
}
