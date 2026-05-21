// Wrapper around Soroban Asset Contract (SAC).

use soroban_sdk::{Address, Env};

pub(crate) fn sac_transfer_call(
    env: &Env,
    token: &Address,
    from: &Address,
    to: &Address,
    amount: &i128,
) {
    soroban_sdk::token::Client::new(env, token).transfer(from, to, amount)
}
