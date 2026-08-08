use soroban_sdk::Env;

use crate::constants::{TTL_BUMP_INSTANCE, TTL_THRESHOLD_INSTANCE};

/// Extend the current contract's instance storage TTL.
///
/// Uses protocol-wide instance thresholds (`TTL_THRESHOLD_INSTANCE` /
/// `TTL_BUMP_INSTANCE`). All contracts should call this rather than inlining
/// the same two-constant `extend_ttl`.
#[inline]
pub fn renew_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD_INSTANCE, TTL_BUMP_INSTANCE);
}
