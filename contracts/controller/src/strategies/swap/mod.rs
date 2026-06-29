//! Aggregator swap execution with balance-delta verification.
//!
//! Strategies do not trust router-reported amounts. Route bytes are opaque;
//! the controller snapshots SAC balances before the router call and verifies
//! input spend and output receipt from observed deltas.

use common::errors::GenericError;
use common::types::StrategySwap;
use soroban_sdk::{panic_with_error, Address, Env};

mod auth;
mod balances;
mod route;

use crate::storage;

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

/// Token balance the controller gained since `balance_before`; a negative delta
/// is impossible from a sane token and panics.
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
