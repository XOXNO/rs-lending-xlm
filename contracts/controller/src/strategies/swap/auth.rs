//! Pre-authorization helper for letting a swap router pull tokens from the
//! current contract.

use common::token::authorize_transfer_as_current;
use soroban_sdk::{Address, Env};

/// Pre-authorizes a `transfer(current_contract, router_addr, amount_in)`
/// call on `token_in`, so the router can pull `amount_in` of `token_in` from
/// the current contract's balance without a separate signature.
pub(crate) fn pre_authorize_router_pull(
    env: &Env,
    router_addr: &Address,
    token_in: &Address,
    amount_in: i128,
) {
    authorize_transfer_as_current(
        env,
        token_in,
        &env.current_contract_address(),
        router_addr,
        amount_in,
    );
}
