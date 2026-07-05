//! Aggregator route execution and route-level validation.

use common::errors::GenericError;
use common::types::StrategySwap;
use soroban_sdk::{assert_with_error, panic_with_error, Env};

use crate::storage;

pub(crate) mod aggregator {
    use soroban_sdk::{contractclient, Address, Bytes, Env};

    #[allow(dead_code)] // Generates the Soroban client proxy.
    #[contractclient(name = "AggregatorClient")]
    pub trait Aggregator {
        fn execute_strategy(env: Env, sender: Address, total_in: i128, swap_xdr: Bytes) -> i128;
    }
}

/// Rejects non-positive amounts and empty swap payloads.
pub(super) fn validate_strategy_swap(env: &Env, swap: &StrategySwap, amount_in: i128) {
    if amount_in <= 0 {
        panic_with_error!(env, GenericError::AmountMustBePositive);
    }
    assert_with_error!(env, !swap.is_empty(), GenericError::InvalidPayments);
}

/// Invokes the aggregator's `execute_strategy` with the flash-loan
/// reentrancy flag set, blocking any reentrant controller call (`supply`,
/// `borrow`, `withdraw`, etc.) for the duration of the swap. `swap` is opaque
/// route XDR decoded only by the aggregator.
pub(super) fn call_router_with_reentrancy_guard(
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
