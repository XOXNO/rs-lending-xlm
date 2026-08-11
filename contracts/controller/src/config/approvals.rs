//! Reads and writes the approval flag that gates whether a Blend pool address
//! is permitted for use by the controller.

use soroban_sdk::{Address, Env};

use crate::events::ApproveBlendPoolEvent;
use crate::storage;

/// Returns whether `pool` is currently marked as an approved Blend pool.
pub(crate) fn is_blend_pool_approved(env: &Env, pool: Address) -> bool {
    storage::is_blend_pool_approved(env, &pool)
}

/// Sets the approval flag for `pool` and publishes an `ApproveBlendPoolEvent`
/// reflecting the new state.
pub(crate) fn set_blend_pool_approval(env: &Env, pool: Address, approved: bool) {
    storage::set_blend_pool_approved(env, &pool, approved);
    ApproveBlendPoolEvent { pool, approved }.publish(env);
}
