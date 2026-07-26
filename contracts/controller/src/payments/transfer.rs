//! Controller-side token movement and balance measurement.
//!
//! Every SAC transfer the controller makes on its own behalf goes through here,
//! so fee-on-transfer and rebasing tokens are handled in one place rather than
//! per call site.

use common::errors::GenericError;
use soroban_sdk::{assert_with_error, panic_with_error, token, Address, Env};

use crate::external::sac::sac_transfer_call;

/// Transfers a positive SAC `amount`, reverting with `non_positive_error` otherwise.
pub(crate) fn transfer_amount(
    env: &Env,
    asset: &Address,
    from: &Address,
    to: &Address,
    amount: i128,
    non_positive_error: GenericError,
) -> i128 {
    assert_with_error!(env, amount > 0, non_positive_error);
    sac_transfer_call(env, asset, from, to, &amount);
    amount
}

/// Like [`transfer_amount`], but returns the balance delta actually credited to
/// `to`, not the nominal `amount`. For a well-behaved SAC the two are equal; a
/// fee-on-transfer or negative-rebase token credits less, so recording the
/// measured delta keeps tracked pool cash from exceeding tokens received.
pub(crate) fn transfer_amount_measured(
    env: &Env,
    asset: &Address,
    from: &Address,
    to: &Address,
    amount: i128,
    non_positive_error: GenericError,
) -> i128 {
    assert_with_error!(env, amount > 0, non_positive_error);
    let tok = token::Client::new(env, asset);
    let pre = tok.balance(to);
    sac_transfer_call(env, asset, from, to, &amount);
    let post = tok.balance(to);
    post.checked_sub(pre)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AmountMustBePositive))
}

/// Token balance the controller gained since `balance_before`; may be negative
/// if the balance decreased. Panics only on i128 overflow, not on an ordinary
/// negative result — every caller checks the sign itself.
pub(crate) fn balance_delta(env: &Env, token: &token::Client, balance_before: i128) -> i128 {
    token
        .balance(&env.current_contract_address())
        .checked_sub(balance_before)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError))
}
