use crate::constants::{
    TTL_BUMP_INSTANCE, TTL_BUMP_SHARED, TTL_BUMP_USER, TTL_THRESHOLD_INSTANCE,
    TTL_THRESHOLD_SHARED, TTL_THRESHOLD_USER,
};
use common::types::ControllerKey;
use soroban_sdk::{Env, IntoVal, TryFromVal, Val};

pub(crate) fn renew_user_key(env: &Env, key: &ControllerKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, TTL_THRESHOLD_USER, TTL_BUMP_USER);
}

fn renew_protocol_shared_key(env: &Env, key: &ControllerKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}

pub(crate) fn renew_controller_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD_INSTANCE, TTL_BUMP_INSTANCE);
}

pub(crate) fn get_shared<V: TryFromVal<Env, Val>>(env: &Env, key: &ControllerKey) -> Option<V> {
    let value: Option<V> = env.storage().persistent().get(key);
    if value.is_some() {
        renew_protocol_shared_key(env, key);
    }
    value
}

pub(crate) fn set_shared<V: IntoVal<Env, Val>>(env: &Env, key: &ControllerKey, value: &V) {
    env.storage().persistent().set(key, value);
    renew_protocol_shared_key(env, key);
}

pub(crate) fn get_user<V: TryFromVal<Env, Val>>(env: &Env, key: &ControllerKey) -> Option<V> {
    let value: Option<V> = env.storage().persistent().get(key);
    if value.is_some() {
        renew_user_key(env, key);
    }
    value
}

pub(crate) fn set_user<V: IntoVal<Env, Val>>(env: &Env, key: &ControllerKey, value: &V) {
    env.storage().persistent().set(key, value);
    renew_user_key(env, key);
}
