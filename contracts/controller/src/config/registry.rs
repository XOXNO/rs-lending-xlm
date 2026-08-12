
use soroban_sdk::{Address, Env};

use crate::events::{
    UpdateAccumulatorEvent, UpdatePriceAggregatorEvent, UpdateSwapAggregatorEvent,
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


use crate::events::ApproveBlendPoolEvent;

pub(crate) fn is_blend_pool_approved(env: &Env, pool: Address) -> bool {
    storage::is_blend_pool_approved(env, &pool)
}

pub(crate) fn set_blend_pool_approval(env: &Env, pool: Address, approved: bool) {
    storage::set_blend_pool_approved(env, &pool, approved);
    ApproveBlendPoolEvent { pool, approved }.publish(env);
}

