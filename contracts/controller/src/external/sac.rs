use soroban_sdk::{Address, Env};

/// Controller-local name for [`common::token::sac_transfer`].
pub(crate) fn sac_transfer_call(
    env: &Env,
    token: &Address,
    from: &Address,
    to: &Address,
    amount: &i128,
) {
    common::token::sac_transfer(env, token, from, to, *amount);
}
