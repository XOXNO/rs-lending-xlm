use common::errors::GenericError;
use soroban_sdk::{assert_with_error, panic_with_error, token, Address, Env};

use crate::external::sac::sac_transfer_call;

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

pub(crate) fn balance_delta(env: &Env, token: &token::Client, balance_before: i128) -> i128 {
    token
        .balance(&env.current_contract_address())
        .checked_sub(balance_before)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError))
}
