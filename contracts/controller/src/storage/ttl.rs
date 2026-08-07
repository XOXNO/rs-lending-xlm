use crate::constants::{
    TTL_BUMP_INSTANCE, TTL_BUMP_SHARED, TTL_BUMP_USER, TTL_THRESHOLD_INSTANCE,
    TTL_THRESHOLD_SHARED, TTL_THRESHOLD_USER,
};
use common::types::ControllerKey;
use soroban_sdk::{Env, IntoVal, TryFromVal, Val};

fn renew_persistent_key(env: &Env, key: &ControllerKey, threshold: u32, bump: u32) {
    env.storage().persistent().extend_ttl(key, threshold, bump);
}

pub(crate) fn renew_user_key(env: &Env, key: &ControllerKey) {
    renew_persistent_key(env, key, TTL_THRESHOLD_USER, TTL_BUMP_USER);
}

pub(crate) fn renew_controller_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD_INSTANCE, TTL_BUMP_INSTANCE);
}

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

pub(crate) fn get_shared<V: TryFromVal<Env, Val>>(env: &Env, key: &ControllerKey) -> Option<V> {
    get_persistent(env, key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED)
}

pub(crate) fn set_shared<V: IntoVal<Env, Val>>(env: &Env, key: &ControllerKey, value: &V) {
    set_persistent(env, key, value, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED)
}

pub(crate) fn get_user<V: TryFromVal<Env, Val>>(env: &Env, key: &ControllerKey) -> Option<V> {
    get_persistent(env, key, TTL_THRESHOLD_USER, TTL_BUMP_USER)
}

pub(crate) fn set_user<V: IntoVal<Env, Val>>(env: &Env, key: &ControllerKey, value: &V) {
    set_persistent(env, key, value, TTL_THRESHOLD_USER, TTL_BUMP_USER)
}
