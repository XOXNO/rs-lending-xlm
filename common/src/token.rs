use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::{
    assert_with_error, panic_with_error, symbol_short, token, vec, Address, Env, IntoVal, Vec,
};

use crate::errors::GenericError;

/// No-op when `amount == 0`; otherwise SAC `transfer(from, to, amount)`.
#[inline]
pub fn sac_transfer(env: &Env, token_addr: &Address, from: &Address, to: &Address, amount: i128) {
    if amount == 0 {
        return;
    }
    token::Client::new(env, token_addr).transfer(from, to, &amount);
}

/// Transfer and credit only what actually arrived.
///
/// Snapshots the recipient balance, performs the transfer, and returns
/// `post - pre` so fee-on-transfer tokens cannot mint unbacked claims.
///
/// `non_positive_error` is raised when `amount <= 0` (callers choose between
/// `AmountMustBePositive` and domain-specific errors such as `InternalError`).
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
    sac_transfer(env, asset, from, to, amount);
    let post = tok.balance(to);
    post.checked_sub(pre)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AmountMustBePositive))
}

/// Authorize the current contract to invoke SAC `transfer(from, to, amount)`.
///
/// Used before nested calls that pull tokens from this contract (swap routers,
/// pool supply, venue hops).
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
