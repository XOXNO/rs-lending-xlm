//! TTL renewal helpers for this contract's three renewal tiers: user and
//! protocol-shared (both Soroban persistent storage, renewed with different
//! threshold/bump constants) and instance (Soroban's native instance tier).

use crate::constants::{
    TTL_BUMP_INSTANCE, TTL_BUMP_SHARED, TTL_BUMP_USER, TTL_THRESHOLD_INSTANCE,
    TTL_THRESHOLD_SHARED, TTL_THRESHOLD_USER,
};
use common::types::ControllerKey;
use soroban_sdk::Env;

/// Extends the user-tier TTL on a persistent key.
pub(crate) fn renew_user_key(env: &Env, key: &ControllerKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, TTL_THRESHOLD_USER, TTL_BUMP_USER);
}

/// Extends the protocol-shared-tier TTL on a persistent key.
pub(crate) fn renew_protocol_shared_key(env: &Env, key: &ControllerKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}

/// Extends the contract's instance-storage TTL.
pub(crate) fn renew_controller_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD_INSTANCE, TTL_BUMP_INSTANCE);
}
