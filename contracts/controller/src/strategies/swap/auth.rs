use common::token::authorize_transfer_as_current;
use soroban_sdk::{Address, Env};

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
