//! Aggregator swaps verified by controller token balance deltas.
//!
//! Router address comes from storage; pull is capped at `amount_in`;
//! underspend refunds to the caller. Reentrancy guard wraps the route call.

use common::errors::GenericError;
use common::types::StrategySwap;
use soroban_sdk::{assert_with_error, token, Address, Env};

mod auth;
mod balances;
mod route;

use crate::storage;
use route::aggregator::AggregatorClient;
use route::validate_strategy_swap;

/// Router from storage only; pull exactly amount_in; verify in/out by balance delta.
pub(crate) fn swap_tokens(
    env: &Env,
    refund_to: &Address,
    token_in: &Address,
    amount_in: i128,
    token_out: &Address,
    swap: &StrategySwap,
) -> i128 {
    // D{token_in.decimals}{Token(token_in)} -> D{token_out.decimals}{Token(token_out)}.
    let router_addr = storage::get_swap_aggregator(env);
    let router = AggregatorClient::new(env, &router_addr);
    let token_out_client = token::Client::new(env, token_out);
    let token_in_client = token::Client::new(env, token_in);

    validate_strategy_swap(env, swap, amount_in);

    let balance_before = balances::snapshot_swap_balances(env, &token_in_client, &token_out_client);

    auth::pre_authorize_router_pull(env, &router_addr, token_in, amount_in);

    route::call_router_with_reentrancy_guard(env, &router, amount_in, swap);

    balances::settle_router_input(
        env,
        &token_in_client,
        balance_before.token_in,
        amount_in,
        refund_to,
    );

    balances::verify_router_output(env, &token_out_client, balance_before.token_out)
}

pub(crate) fn swap_tokens_or_passthrough(
    env: &Env,
    refund_to: &Address,
    token_in: &Address,
    amount_in: i128,
    token_out: &Address,
    swap: &StrategySwap,
) -> i128 {
    if token_in == token_out {
        assert_with_error!(env, swap.is_empty(), GenericError::InvalidPayments);
        amount_in
    } else {
        swap_tokens(env, refund_to, token_in, amount_in, token_out, swap)
    }
}
