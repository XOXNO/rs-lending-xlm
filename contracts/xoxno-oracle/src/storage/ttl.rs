//! TTL renewal helpers for the oracle contract's instance and persistent
//! storage entries.

use common::constants::{TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};
use soroban_sdk::Env;

use crate::storage::DataKey;

/// Extends the contract's instance storage TTL using the protocol-wide
/// instance constants `TTL_THRESHOLD_INSTANCE` / `TTL_BUMP_INSTANCE`.
pub(crate) fn renew_oracle_instance(env: &Env) {
    common::ttl::renew_instance(env);
}

/// Extends `key`'s persistent storage TTL using `TTL_THRESHOLD_SHARED` and `TTL_BUMP_SHARED`.
pub(crate) fn renew_persistent_key(env: &Env, key: &DataKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}
