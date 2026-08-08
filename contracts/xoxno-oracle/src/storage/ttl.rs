use common::constants::{TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};
use soroban_sdk::Env;

use crate::storage::DataKey;

pub(crate) fn renew_oracle_instance(env: &Env) {
    common::ttl::renew_instance(env);
}

pub(crate) fn renew_persistent_key(env: &Env, key: &DataKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}
