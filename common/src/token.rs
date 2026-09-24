//! Token helpers: a transfer that measures the amount received, and an
//! authorization entry for a token `transfer` made on the current contract's behalf.

use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::{
    assert_with_error, panic_with_error, symbol_short, token, vec, Address, Env, IntoVal, Vec,
};

use crate::errors::GenericError;

/// Transfers `amount` from `from` to `to` and returns the balance change at
/// `to` (`post - pre`, token amount). The result can differ from `amount`.
///
/// Panics with `non_positive_error` if `amount <= 0`, and with
/// `GenericError::AmountMustBePositive` if `post - pre` overflows `i128`.
pub fn transfer_amount_measured(
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
    tok.transfer(from, to, &amount);
    let post = tok.balance(to);
    post.checked_sub(pre)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AmountMustBePositive))
}

/// Authorizes, on behalf of the current contract, one `transfer(from, to, amount)`
/// call on `token_addr` made deeper in the next contract call (for example by
/// the pool). The entry allows no further sub-invocations.
pub fn authorize_transfer_as_current(
    env: &Env,
    token_addr: &Address,
    from: &Address,
    to: &Address,
    amount: i128,
) {
    let entry = InvokerContractAuthEntry::Contract(SubContractInvocation {
        context: ContractContext {
            contract: token_addr.clone(),
            fn_name: symbol_short!("transfer"),
            args: (from.clone(), to.clone(), amount).into_val(env),
        },
        sub_invocations: Vec::new(env),
    });
    env.authorize_as_current_contract(vec![env, entry]);
}

#[cfg(test)]
#[path = "../tests/token.rs"]
mod tests;
