//! TTL renewal helpers for this contract's three renewal tiers: user and
//! protocol-shared (both Soroban persistent storage, renewed with different
//! threshold/bump constants) and instance (Soroban's native instance tier).

use crate::constants::{
    TTL_BUMP_INSTANCE, TTL_BUMP_SHARED, TTL_BUMP_USER, TTL_THRESHOLD_INSTANCE,
    TTL_THRESHOLD_SHARED, TTL_THRESHOLD_USER,
};
use common::types::ControllerKey;
use soroban_sdk::{Env, IntoVal, TryFromVal, Val};

/// Extends the user-tier TTL on a persistent key.
pub(crate) fn renew_user_key(env: &Env, key: &ControllerKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, TTL_THRESHOLD_USER, TTL_BUMP_USER);
}

/// Extends the protocol-shared-tier TTL on a persistent key.
fn renew_protocol_shared_key(env: &Env, key: &ControllerKey) {
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

/// Reads a protocol-shared key, renewing its TTL when the entry exists.
///
/// Read-renewal keeps stable protocol state (hubs, spokes, listings, usage)
/// from archiving while accounts still depend on it. A miss renews nothing, so
/// probing an absent key cannot resurrect it.
pub(crate) fn get_shared<V: TryFromVal<Env, Val>>(env: &Env, key: &ControllerKey) -> Option<V> {
    let value: Option<V> = env.storage().persistent().get(key);
    if value.is_some() {
        renew_protocol_shared_key(env, key);
    }
    value
}

/// Writes a protocol-shared key and renews its TTL.
pub(crate) fn set_shared<V: IntoVal<Env, Val>>(env: &Env, key: &ControllerKey, value: &V) {
    env.storage().persistent().set(key, value);
    renew_protocol_shared_key(env, key);
}

/// Reads a user-tier key, renewing its TTL when the entry exists.
pub(crate) fn get_user<V: TryFromVal<Env, Val>>(env: &Env, key: &ControllerKey) -> Option<V> {
    let value: Option<V> = env.storage().persistent().get(key);
    if value.is_some() {
        renew_user_key(env, key);
    }
    value
}

/// Writes a user-tier key and renews its TTL.
pub(crate) fn set_user<V: IntoVal<Env, Val>>(env: &Env, key: &ControllerKey, value: &V) {
    env.storage().persistent().set(key, value);
    renew_user_key(env, key);
}
