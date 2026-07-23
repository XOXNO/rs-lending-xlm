//! Protocol registry setters: swap aggregator, price aggregator, accumulator,
//! and the liquidity-pool WASM template.

use soroban_sdk::{Address, BytesN, Env};

use crate::events::{
    UpdateAccumulatorEvent, UpdatePoolTemplateEvent, UpdatePriceAggregatorEvent,
    UpdateSwapAggregatorEvent,
};
use crate::storage;

pub(crate) fn set_swap_aggregator(env: &Env, addr: Address) {
    storage::set_swap_aggregator(env, &addr);
    UpdateSwapAggregatorEvent {
        swap_aggregator: addr,
    }
    .publish(env);
}

pub(crate) fn set_price_aggregator(env: &Env, addr: Address) {
    storage::set_price_aggregator(env, &addr);
    UpdatePriceAggregatorEvent {
        price_aggregator: addr,
    }
    .publish(env);
}

pub(crate) fn set_accumulator(env: &Env, addr: Address) {
    storage::set_accumulator(env, &addr);
    UpdateAccumulatorEvent { accumulator: addr }.publish(env);
}

pub(crate) fn set_liquidity_pool_template(env: &Env, hash: BytesN<32>) {
    storage::set_pool_template(env, &hash);
    UpdatePoolTemplateEvent { wasm_hash: hash }.publish(env);
}
