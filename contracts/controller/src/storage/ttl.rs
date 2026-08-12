//! Read and write helpers for controller persistent storage that extend each entry's TTL
//! according to per-category (shared vs. user) threshold and bump constants.

use crate::constants::{TTL_BUMP_SHARED, TTL_BUMP_USER, TTL_THRESHOLD_SHARED, TTL_THRESHOLD_USER};
use common::types::ControllerKey;
use soroban_sdk::{Env, IntoVal, TryFromVal, Val};

/// Extends the TTL of the persistent storage entry at `key` to `bump` ledgers once its
/// remaining TTL falls to or below `threshold`.
fn renew_persistent_key(env: &Env, key: &ControllerKey, threshold: u32, bump: u32) {
    env.storage().persistent().extend_ttl(key, threshold, bump);
}

/// Extends the TTL of a user-scoped persistent storage entry using the user TTL threshold
/// and bump constants.
pub(crate) fn renew_user_key(env: &Env, key: &ControllerKey) {
    renew_persistent_key(env, key, TTL_THRESHOLD_USER, TTL_BUMP_USER);
}

/// Extends the TTL of the contract instance's storage.
pub(crate) fn renew_controller_instance(env: &Env) {
    common::ttl::renew_instance(env);
}

/// Reads the persistent storage value at `key`. If the key is present, extends its TTL
/// using `threshold` and `bump`. Returns `None` if the key is absent.
fn get_persistent<V: TryFromVal<Env, Val>>(
    env: &Env,
    key: &ControllerKey,
    threshold: u32,
    bump: u32,
) -> Option<V> {
    let value: Option<V> = env.storage().persistent().get(key);
    if value.is_some() {
        renew_persistent_key(env, key, threshold, bump);
    }
    value
}

/// Writes `value` to persistent storage at `key` and extends its TTL using `threshold`
/// and `bump`.
fn set_persistent<V: IntoVal<Env, Val>>(
    env: &Env,
    key: &ControllerKey,
    value: &V,
    threshold: u32,
    bump: u32,
) {
    env.storage().persistent().set(key, value);
    renew_persistent_key(env, key, threshold, bump);
}

/// Reads a shared persistent storage value at `key`, extending its TTL using the shared
/// TTL threshold and bump constants if the key is present. Returns `None` if the key is
/// absent.
pub(crate) fn get_shared<V: TryFromVal<Env, Val>>(env: &Env, key: &ControllerKey) -> Option<V> {
    get_persistent(env, key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED)
}

/// Writes `value` to shared persistent storage at `key` and extends its TTL using the
/// shared TTL threshold and bump constants.
pub(crate) fn set_shared<V: IntoVal<Env, Val>>(env: &Env, key: &ControllerKey, value: &V) {
    set_persistent(env, key, value, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED)
}

/// Reads a user-scoped persistent storage value at `key`, extending its TTL using the user
/// TTL threshold and bump constants if the key is present. Returns `None` if the key is
/// absent.
pub(crate) fn get_user<V: TryFromVal<Env, Val>>(env: &Env, key: &ControllerKey) -> Option<V> {
    get_persistent(env, key, TTL_THRESHOLD_USER, TTL_BUMP_USER)
}

/// Writes `value` to user-scoped persistent storage at `key` and extends its TTL using the
/// user TTL threshold and bump constants.
pub(crate) fn set_user<V: IntoVal<Env, Val>>(env: &Env, key: &ControllerKey, value: &V) {
    set_persistent(env, key, value, TTL_THRESHOLD_USER, TTL_BUMP_USER)
}
