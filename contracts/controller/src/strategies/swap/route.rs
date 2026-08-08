use common::errors::GenericError;
use common::types::StrategySwap;
use common::validation::require_positive_amount;

use soroban_sdk::{assert_with_error, Env};

use crate::storage;

pub(crate) mod aggregator {
    use soroban_sdk::{contractclient, Address, Bytes, Env};

    #[allow(dead_code)]
    #[contractclient(name = "AggregatorClient")]
    pub trait Aggregator {
        fn execute_strategy(env: Env, sender: Address, total_in: i128, swap_xdr: Bytes) -> i128;
    }
}

pub(crate) fn validate_strategy_swap(env: &Env, swap: &StrategySwap, amount_in: i128) {
    require_positive_amount(env, amount_in);
    assert_with_error!(env, !swap.is_empty(), GenericError::InvalidPayments);
}

pub(crate) fn call_router_with_reentrancy_guard(
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
