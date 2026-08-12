//! Helper for renewing a contract's instance storage TTL using protocol-wide
//! threshold and bump constants.

use soroban_sdk::Env;

use crate::constants::{TTL_BUMP_INSTANCE, TTL_THRESHOLD_INSTANCE};

/// Extends the current contract's instance storage TTL using
/// `TTL_THRESHOLD_INSTANCE` and `TTL_BUMP_INSTANCE`.
#[inline]
pub fn renew_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD_INSTANCE, TTL_BUMP_INSTANCE);
}
