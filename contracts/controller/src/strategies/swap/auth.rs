use common::token::authorize_transfer_as_current;
use soroban_sdk::{Address, Env};

/// Pre-authorizes a `token_in.transfer` from this contract to `router_addr` for
/// `amount_in`, letting the router pull the swap input without a further signature.
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
