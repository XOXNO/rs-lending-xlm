use common::errors::GenericError;
use common::types::StrategySwap;
use common::validation::require_positive_amount;

use soroban_sdk::{assert_with_error, Env};
use swap_aggregator_interface::SwapAggregatorClient;

use crate::storage;

pub(crate) fn validate_strategy_swap(env: &Env, swap: &StrategySwap, amount_in: i128) {
    require_positive_amount(env, amount_in);
    assert_with_error!(env, !swap.is_empty(), GenericError::InvalidPayments);
}

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
