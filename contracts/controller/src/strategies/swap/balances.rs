use common::errors::{GenericError, StrategyError};
use soroban_sdk::{assert_with_error, panic_with_error, token, Address, Env};

pub(crate) struct SwapBalanceSnapshot {
    pub(crate) token_in: i128,

    pub(crate) token_out: i128,
}

/// Reads this contract's current `token_in` and `token_out` balances into a
/// snapshot for later before/after comparison.
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

/// Verifies the router spent no more than `amount_in` of `token_in` from this
/// contract's balance, panicking with `RouterOverspend` if it spent more or the
/// balance increased. Refunds any unspent leftover to `refund_to`.
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

/// Computes the increase in this contract's `token_out` balance since
/// `balance_before` and returns it, panicking with `NoSwapOutput` if no output
/// was received.
pub(crate) fn verify_router_output(
    env: &Env,
    token_out_client: &token::Client,
    balance_before: i128,
) -> i128 {
    let received = token_out_client
        .balance(&env.current_contract_address())
        .checked_sub(balance_before)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError));
    assert_with_error!(env, received > 0, StrategyError::NoSwapOutput);
    received
}
