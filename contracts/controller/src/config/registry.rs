//! Sets the addresses of the swap aggregator, price aggregator, and
//! accumulator contracts the controller delegates to.

use soroban_sdk::{Address, Env};

use crate::events::{
    UpdateAccumulatorEvent, UpdatePriceAggregatorEvent, UpdateSwapAggregatorEvent,
};
use crate::storage;

/// Sets the swap aggregator address and publishes an
/// `UpdateSwapAggregatorEvent`.
pub(crate) fn set_swap_aggregator(env: &Env, addr: Address) {
    storage::set_swap_aggregator(env, &addr);
    UpdateSwapAggregatorEvent {
        swap_aggregator: addr,
    }
    .publish(env);
}

/// Sets the price aggregator address and publishes an
/// `UpdatePriceAggregatorEvent`.
pub(crate) fn set_price_aggregator(env: &Env, addr: Address) {
    storage::set_price_aggregator(env, &addr);
    UpdatePriceAggregatorEvent {
        price_aggregator: addr,
    }
    .publish(env);
}

/// Sets the accumulator address and publishes an `UpdateAccumulatorEvent`.
pub(crate) fn set_accumulator(env: &Env, addr: Address) {
    storage::set_accumulator(env, &addr);
    UpdateAccumulatorEvent { accumulator: addr }.publish(env);
}
