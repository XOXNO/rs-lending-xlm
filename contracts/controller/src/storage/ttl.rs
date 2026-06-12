//! TTL renewal helpers for Soroban's three storage rent tiers (user,
//! protocol-shared, instance).

use crate::constants::{
    TTL_BUMP_INSTANCE, TTL_BUMP_SHARED, TTL_BUMP_USER, TTL_THRESHOLD_INSTANCE,
    TTL_THRESHOLD_SHARED, TTL_THRESHOLD_USER,
};
use controller_interface::types::ControllerKey;
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
