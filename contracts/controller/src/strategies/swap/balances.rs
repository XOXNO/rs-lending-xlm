//! Balance snapshotting and reconciliation for swaps executed through an
//! external router.

use common::errors::{GenericError, StrategyError};
use soroban_sdk::{assert_with_error, panic_with_error, token, Address, Env};


/// Current contract's token-in and token-out balances captured before a
/// router swap.
pub(crate) struct SwapBalanceSnapshot {
    pub(crate) token_in: i128,

    pub(crate) token_out: i128,
}

/// Reads the current contract's token-in and token-out balances.
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

/// Reconciles token-in spend after a router call. Refunds the unspent
/// remainder of `amount_in` to `refund_to`. Panics with
/// `StrategyError::RouterOverspend` if the current contract's token-in
/// balance increased, or if the router spent more than `amount_in`.
pub(crate) fn settle_router_input(
    env: &Env,
    token_in_client: &token::Client,
    balance_before: i128,
    amount_in: i128,
    refund_to: &Address,
) {
    let balance_after = token_in_client.balance(&env.current_contract_address());
    assert_with_error!(
        env,
        balance_after <= balance_before,
        StrategyError::RouterOverspend
    );

    let actual_spent = balance_before - balance_after;
    assert_with_error!(
        env,
        actual_spent <= amount_in,
        StrategyError::RouterOverspend
    );

    let leftover = amount_in - actual_spent;
    if leftover > 0 {
        token_in_client.transfer(&env.current_contract_address(), refund_to, &leftover);
    }
}

/// Returns the increase in the current contract's token-out balance since
/// `balance_before`. Panics with `StrategyError::NoSwapOutput` if the
/// balance did not increase.
pub(crate) fn verify_router_output(
    env: &Env,
    token_out_client: &token::Client,
    balance_before: i128,
) -> i128 {
    let received = token_out_client.balance(&env.current_contract_address()).checked_sub(balance_before).unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError));
    assert_with_error!(env, received > 0, StrategyError::NoSwapOutput);
    received
}
