use common::errors::GenericError;
use common::types::StrategySwap;
use common::validation::require_positive_amount;

use soroban_sdk::{assert_with_error, Env};
use swap_aggregator_interface::SwapAggregatorClient;

use crate::storage;

/// Checks that `amount_in` is positive and that `swap` carries a non-empty
/// route, panicking with `InvalidPayments` if the route is empty.
pub(crate) fn validate_strategy_swap(env: &Env, swap: &StrategySwap, amount_in: i128) {
    require_positive_amount(env, amount_in);
    assert_with_error!(env, !swap.is_empty(), GenericError::InvalidPayments);
}

/// Calls the aggregator router's `execute_strategy` with this contract as
/// sender while the flash-loan guard flag is held, discarding the returned
/// amount.
pub(crate) fn call_router_with_reentrancy_guard(
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
