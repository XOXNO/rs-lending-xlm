//! Executes swaps through the configured swap aggregator router, validating
//! the swap route, pre-authorizing the router's token pull, and reconciling
//! balances before and after the router call.

use common::errors::GenericError;
use common::types::StrategySwap;
use soroban_sdk::{assert_with_error, token, Address, Env};

mod auth;
mod balances;
mod route;

use crate::storage;
use route::validate_strategy_swap;
use swap_aggregator_interface::SwapAggregatorClient;

/// Swaps `amount_in` of `token_in` for `token_out` through the configured
/// swap aggregator router and returns the amount of `token_out` received.
///
/// Validates `swap`, snapshots balances, pre-authorizes the router to pull
/// `amount_in` of `token_in`, invokes the router under a reentrancy guard,
/// refunds any unspent `token_in` to `refund_to`, and verifies that
/// `token_out` was actually received.
pub(crate) fn swap_tokens(
    env: &Env,
    refund_to: &Address,
    token_in: &Address,
    amount_in: i128,
    token_out: &Address,
    swap: &StrategySwap,
) -> i128 {
    let router_addr = storage::get_swap_aggregator(env);
    let router = SwapAggregatorClient::new(env, &router_addr);
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

/// Returns `amount_in` unchanged when `token_in` and `token_out` are the
/// same asset, otherwise delegates to `swap_tokens`. Panics with
/// `GenericError::InvalidPayments` if `token_in` equals `token_out` but
/// `swap` is non-empty.
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
