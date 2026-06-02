//! TTL renewal helpers — the three tiers of Soroban storage rent.
//!
//! User (per-account) keys get shorter, cheaper bumps; protocol-shared keys
//! (markets, pools list, e-mode, isolated debt) get longer bumps since many
//! accounts read them; the controller instance is bumped on every mutating
//! entrypoint via `renew_controller_instance` (from `Cache::new`). Centralizing
//! the constants and call sites here keeps rent strategy easy to adjust.

use common::constants::{
    TTL_BUMP_INSTANCE, TTL_BUMP_SHARED, TTL_BUMP_USER, TTL_THRESHOLD_INSTANCE,
    TTL_THRESHOLD_SHARED, TTL_THRESHOLD_USER,
};
use common::types::ControllerKey;
use soroban_sdk::Env;

pub(crate) fn renew_user_key(env: &Env, key: &ControllerKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, TTL_THRESHOLD_USER, TTL_BUMP_USER);
}

pub(crate) fn renew_protocol_shared_key(env: &Env, key: &ControllerKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}

pub(crate) fn renew_controller_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD_INSTANCE, TTL_BUMP_INSTANCE);
}
