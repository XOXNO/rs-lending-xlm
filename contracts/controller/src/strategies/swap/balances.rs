//! Verifies router spend/output by controller SAC balance deltas.

use common::errors::{GenericError, StrategyError};
use soroban_sdk::{assert_with_error, panic_with_error, token, Address, Env};

use crate::strategies::swap::balance_delta;

pub(crate) struct SwapBalanceSnapshot {
    // D{token_in.decimals}{Token(token_in)} controller balance before router call.
    pub(crate) token_in: i128,
    // D{token_out.decimals}{Token(token_out)} controller balance before router call.
    pub(crate) token_out: i128,
}

pub(crate) fn snapshot_swap_balances(
    env: &Env,
    token_in_client: &token::Client,
    token_out_client: &token::Client,
) -> SwapBalanceSnapshot {
    SwapBalanceSnapshot {
        token_in: token_in_client.balance(&env.current_contract_address()),
        token_out: token_out_client.balance(&env.current_contract_address()),
    }
}

pub(crate) fn verify_router_input_spend(
    env: &Env,
    token_in_client: &token::Client,
    balance_before: i128,
    amount_in: i128,
) {
    let balance_after = token_in_client.balance(&env.current_contract_address());
    assert_with_error!(
        env,
        balance_after <= balance_before,
        StrategyError::RouterOverspend
    );
    // D{token_in.decimals}{Token(token_in)} spent by router from controller balance.
    let actual_in_spent = balance_before - balance_after;
    assert_with_error!(
        env,
        actual_in_spent <= amount_in,
        StrategyError::RouterOverspend
    );
}

pub(crate) fn refund_router_underspend(
    env: &Env,
    token_in_client: &token::Client,
    balance_before: i128,
    amount_in: i128,
    refund_to: &Address,
) {
    let balance_after = token_in_client.balance(&env.current_contract_address());
    let actual_spent = balance_before
        .checked_sub(balance_after)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError));
    // D{token_in.decimals}{Token(token_in)} refund router underspend in same input token.
    let leftover = amount_in
        .checked_sub(actual_spent)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError));
    if leftover > 0 {
        token_in_client.transfer(&env.current_contract_address(), refund_to, &leftover);
    }
}

pub(crate) fn verify_router_output(
    env: &Env,
    token_out_client: &token::Client,
    balance_before: i128,
) -> i128 {
    // D{token_out.decimals}{Token(token_out)} verified router output by balance delta.
    let received = balance_delta(env, token_out_client, balance_before);
    assert_with_error!(env, received > 0, StrategyError::NoSwapOutput);
    received
}
