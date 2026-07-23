//! Blend-pool approval allowlist for migration.

use soroban_sdk::{Address, Env};

use crate::events::ApproveBlendPoolEvent;
use crate::storage;

pub(crate) fn is_blend_pool_approved(env: &Env, pool: Address) -> bool {
    storage::is_blend_pool_approved(env, &pool)
}

pub(crate) fn set_blend_pool_approval(env: &Env, pool: Address, approved: bool) {
    storage::set_blend_pool_approved(env, &pool, approved);
    ApproveBlendPoolEvent { pool, approved }.publish(env);
}
