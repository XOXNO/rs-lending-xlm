//! Soroban Asset Contract transfer wrapper used by controller flows.

use soroban_sdk::{token, Address, Env};

/// Transfers `amount` of `token` from `from` to `to` via the SAC client.
pub(crate) fn sac_transfer_call(
    env: &Env,
    token: &Address,
    from: &Address,
    to: &Address,
    amount: &i128,
) {
    token::Client::new(env, token).transfer(from, to, amount)
}
