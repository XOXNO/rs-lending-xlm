//! Aggregator swap execution with balance-delta verification.
//!
//! Strategies do not trust router-reported amounts. Route bytes are opaque;
//! the controller snapshots SAC balances before the router call and verifies
//! input spend and output receipt from observed deltas.

use common::errors::GenericError;
use controller_interface::types::StrategySwap;
use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::{assert_with_error, panic_with_error, symbol_short, Address, Env, IntoVal, Vec};

use crate::storage;

pub(crate) mod aggregator {
    use soroban_sdk::{contractclient, Address, Bytes, Env};

    #[allow(dead_code)] // Generates the Soroban client proxy.
    #[contractclient(name = "AggregatorClient")]
    pub trait Aggregator {
        fn execute_strategy(env: Env, sender: Address, total_in: i128, swap_xdr: Bytes) -> i128;
    }
}

struct SwapBalanceSnapshot {
    // D{token_in.decimals}{Token(token_in)} controller balance before router call.
    token_in: i128,
    // D{token_out.decimals}{Token(token_out)} controller balance before router call.
    token_out: i128,
}

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
    let router = aggregator::AggregatorClient::new(env, &router_addr);
    let token_out_client = soroban_sdk::token::Client::new(env, token_out);
    let token_in_client = soroban_sdk::token::Client::new(env, token_in);

    validate_strategy_swap(env, swap, amount_in);

    let balance_before = snapshot_swap_balances(env, &token_in_client, &token_out_client);

    pre_authorize_router_pull(env, &router_addr, token_in, amount_in);

    call_router_with_reentrancy_guard(env, &router, amount_in, swap);

    verify_router_input_spend(env, &token_in_client, balance_before.token_in, amount_in);
    refund_router_underspend(
        env,
        &token_in_client,
        balance_before.token_in,
        amount_in,
        refund_to,
    );

    verify_router_output(env, &token_out_client, balance_before.token_out)
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

fn validate_strategy_swap(env: &Env, swap: &StrategySwap, amount_in: i128) {
    if amount_in <= 0 {
        panic_with_error!(env, GenericError::AmountMustBePositive);
    }
    assert_with_error!(env, !swap.is_empty(), GenericError::InvalidPayments);
}

fn snapshot_swap_balances(
    env: &Env,
    token_in_client: &soroban_sdk::token::Client,
    token_out_client: &soroban_sdk::token::Client,
) -> SwapBalanceSnapshot {
    SwapBalanceSnapshot {
        token_in: token_in_client.balance(&env.current_contract_address()),
        token_out: token_out_client.balance(&env.current_contract_address()),
    }
}

fn call_router_with_reentrancy_guard(
    env: &Env,
    router: &aggregator::AggregatorClient,
    amount_in: i128,
    swap: &StrategySwap,
) {
    storage::with_flash_guard(env, || {
        let sender = env.current_contract_address();
        let _ = router.execute_strategy(&sender, &amount_in, swap);
    });
}

fn pre_authorize_router_pull(
    env: &Env,
    router_addr: &Address,
    token_in: &Address,
    amount_in: i128,
) {
    let entry = InvokerContractAuthEntry::Contract(SubContractInvocation {
        context: ContractContext {
            contract: token_in.clone(),
            fn_name: symbol_short!("transfer"),
            args: (
                env.current_contract_address(),
                router_addr.clone(),
                amount_in,
            )
                .into_val(env),
        },
        sub_invocations: Vec::new(env),
    });
    env.authorize_as_current_contract(soroban_sdk::vec![env, entry]);
}

fn verify_router_input_spend(
    env: &Env,
    token_in_client: &soroban_sdk::token::Client,
    balance_before: i128,
    amount_in: i128,
) {
    let balance_after = token_in_client.balance(&env.current_contract_address());
    assert_with_error!(
        env,
        balance_after <= balance_before,
        GenericError::InternalError
    );
    // D{token_in.decimals}{Token(token_in)} spent by router from controller balance.
    let actual_in_spent = balance_before - balance_after;
    assert_with_error!(
        env,
        actual_in_spent <= amount_in,
        GenericError::InternalError
    );
}

fn refund_router_underspend(
    env: &Env,
    token_in_client: &soroban_sdk::token::Client,
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

fn verify_router_output(
    env: &Env,
    token_out_client: &soroban_sdk::token::Client,
    balance_before: i128,
) -> i128 {
    // D{token_out.decimals}{Token(token_out)} verified router output by balance delta.
    let received = balance_delta(env, token_out_client, balance_before);
    assert_with_error!(env, received > 0, GenericError::InternalError);
    received
}
