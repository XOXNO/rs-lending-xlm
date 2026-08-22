//! TTL renewal helper for the oracle contract's persistent storage entries.

use common::constants::{TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};
use soroban_sdk::Env;

use crate::storage::DataKey;

/// Extends `key`'s persistent storage TTL using `TTL_THRESHOLD_SHARED` and `TTL_BUMP_SHARED`.
pub(in crate::storage) fn renew_persistent_key(env: &Env, key: &DataKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}
