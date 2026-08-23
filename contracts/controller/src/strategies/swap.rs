use common::errors::{GenericError, StrategyError};
use common::token::authorize_transfer_as_current;
use common::types::StrategySwap;
use common::validation::require_positive_amount;
use soroban_sdk::{assert_with_error, token, Address, Env};
use swap_aggregator_interface::SwapAggregatorClient;

use crate::payments::balance_delta_since;
use crate::storage;

/// Executes a swap of `amount_in` of `token_in` into `token_out` through the
/// configured aggregator router under the flash-loan reentrancy guard. Refunds
/// any unspent `token_in` to `refund_to` and returns the amount of `token_out`
/// actually received.
pub(crate) fn swap_tokens(
    env: &Env,
    refund_to: &Address,
    token_in: &Address,
    amount_in: i128,
    token_out: &Address,
    swap: &StrategySwap,
) -> i128 {
    require_positive_amount(env, amount_in);
    assert_with_error!(env, !swap.is_empty(), GenericError::InvalidPayments);

    let controller = env.current_contract_address();
    let router_addr = storage::get_swap_aggregator(env);
    let router = SwapAggregatorClient::new(env, &router_addr);
    let token_in_client = token::Client::new(env, token_in);

    // Both sides are snapshotted before anything external runs: the input
    // balance bounds what the router may spend, and the output balance is the
    // baseline `verify_router_output` measures against after the call.
    let in_before = token_in_client.balance(&controller);
    let out_before = token::Client::new(env, token_out).balance(&controller);

    // Exact-amount pull authorization, scoped to this router and this amount,
    // so the router can take the input without a further signature.
    authorize_transfer_as_current(env, token_in, &controller, &router_addr, amount_in);

    call_router_with_reentrancy_guard(env, &router, amount_in, swap);

    // Input settlement: the router must not have spent more than `amount_in`,
    // and whatever it left behind goes back to `refund_to`.
    let in_after = token_in_client.balance(&controller);
    assert_with_error!(env, in_after <= in_before, StrategyError::RouterOverspend);
    let actual_spent = in_before - in_after;
    assert_with_error!(
        env,
        actual_spent <= amount_in,
        StrategyError::RouterOverspend
    );
    let leftover = amount_in - actual_spent;
    if leftover > 0 {
        token_in_client.transfer(&controller, refund_to, &leftover);
    }

    verify_router_output(env, token_out, out_before)
}

/// Returns `amount_in` unchanged if `token_in` and `token_out` are the same
/// asset, requiring `swap` to be empty in that case; otherwise routes the
/// amount through [`swap_tokens`].
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

/// Calls the aggregator router's `execute_strategy` with this contract as
/// sender while the flash-loan guard flag is held, discarding the returned
/// amount.
fn call_router_with_reentrancy_guard(
    env: &Env,
    router: &SwapAggregatorClient,
    amount_in: i128,
    swap: &StrategySwap,
) {
    storage::with_flash_guard(env, || {
        let sender = env.current_contract_address();
        let _ = router.execute_strategy(&sender, &amount_in, swap);
    });
}

/// Computes the increase in this contract's `token_out` balance since
/// `balance_before` and returns it, panicking with `NoSwapOutput` if no output
/// was received.
fn verify_router_output(env: &Env, token_out: &Address, balance_before: i128) -> i128 {
    let received = balance_delta_since(
        env,
        token_out,
        &env.current_contract_address(),
        balance_before,
    );
    assert_with_error!(env, received > 0, StrategyError::NoSwapOutput);
    received
}
