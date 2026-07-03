//! Aggregator swaps verified by token balance deltas.

use common::errors::GenericError;
use common::types::StrategySwap;
use soroban_sdk::{panic_with_error, Address, Env};

mod auth;
mod balances;
mod route;

use crate::storage;

/// Swaps `amount_in` of `token_in` for `token_out` through the
/// governance-configured aggregator router. The router address is read from
/// storage, never supplied by the caller; `swap` is opaque route XDR that
/// only the router decodes, and slippage/min-out enforcement lives inside
/// the router's `execute_strategy`, not here. This function only
/// pre-authorizes the router to pull exactly `amount_in` of `token_in`, then
/// verifies by balance delta that the router did not overspend the input and
/// that some output was received.
pub(crate) fn swap_tokens(
    env: &Env,
    refund_to: &Address,
    token_in: &Address,
    amount_in: i128,
    token_out: &Address,
    swap: &StrategySwap,
) -> i128 {
    // D{token_in.decimals}{Token(token_in)} -> D{token_out.decimals}{Token(token_out)}.
    let router_addr = storage::get_aggregator(env);
    let router = route::aggregator::AggregatorClient::new(env, &router_addr);
    let token_out_client = soroban_sdk::token::Client::new(env, token_out);
    let token_in_client = soroban_sdk::token::Client::new(env, token_in);

    route::validate_strategy_swap(env, swap, amount_in);

    let balance_before = balances::snapshot_swap_balances(env, &token_in_client, &token_out_client);

    auth::pre_authorize_router_pull(env, &router_addr, token_in, amount_in);

    route::call_router_with_reentrancy_guard(env, &router, amount_in, swap);

    balances::verify_router_input_spend(env, &token_in_client, balance_before.token_in, amount_in);
    balances::refund_router_underspend(
        env,
        &token_in_client,
        balance_before.token_in,
        amount_in,
        refund_to,
    );

    balances::verify_router_output(env, &token_out_client, balance_before.token_out)
}

/// Token balance the controller gained since `balance_before`; may be
/// negative if the balance decreased. Panics only on i128 overflow, not on
/// an ordinary negative result — every caller checks the sign itself.
pub(crate) fn balance_delta(
    env: &Env,
    token: &soroban_sdk::token::Client,
    balance_before: i128,
) -> i128 {
    token
        .balance(&env.current_contract_address())
        .checked_sub(balance_before)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError))
}
