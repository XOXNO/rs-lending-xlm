//! Registry setters for the aggregator, accumulator, pool template, and the
//! token and Blend-pool approval allowlists.

use soroban_sdk::{xdr::ToXdr, Address, BytesN, Env};

use crate::events::{
    ApproveBlendPoolEvent, ApproveTokenEvent, UpdateAccumulatorEvent, UpdateAggregatorEvent,
    UpdatePoolTemplateEvent,
};
use crate::storage;

/// Stores the swap-aggregator address and emits the update event.
pub(crate) fn set_aggregator(env: &Env, addr: Address) {
    storage::set_aggregator(env, &addr);
    UpdateAggregatorEvent { aggregator: addr }.publish(env);
}

/// Stores the revenue-accumulator address and emits the update event.
pub(crate) fn set_accumulator(env: &Env, addr: Address) {
    storage::set_accumulator(env, &addr);
    UpdateAccumulatorEvent { accumulator: addr }.publish(env);
}

/// Stores the pool-template Wasm hash and emits the update event.
pub(crate) fn set_liquidity_pool_template(env: &Env, hash: BytesN<32>) {
    storage::set_pool_template(env, &hash);
    UpdatePoolTemplateEvent { wasm_hash: hash }.publish(env);
}

/// Sets a token's market-creation approval flag and emits the approval event.
pub(crate) fn set_token_approval(env: &Env, token: Address, approved: bool) {
    storage::renew_controller_instance(env);
    storage::set_token_approved(env, &token, approved);
    let wasm_hash = env.crypto().keccak256(&token.to_xdr(env)).into();
    ApproveTokenEvent {
        wasm_hash,
        approved,
    }
    .publish(env);
}

/// Returns whether the Blend pool is on the migration allowlist.
pub(crate) fn is_blend_pool_approved(env: &Env, pool: Address) -> bool {
    storage::is_blend_pool_approved(env, &pool)
}

/// Sets a Blend pool's migration-allowlist flag and emits the approval event.
pub(crate) fn set_blend_pool_approval(env: &Env, pool: Address, approved: bool) {
    storage::set_blend_pool_approved(env, &pool, approved);
    ApproveBlendPoolEvent { pool, approved }.publish(env);
}
