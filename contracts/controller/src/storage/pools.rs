use super::renew_protocol_shared_key;
use common::constants::MAX_POOLS_LIST_ENTRIES;
use common::errors::GenericError;
use common::types::ControllerKey;
use soroban_sdk::{assert_with_error, Address, Env, Vec};

// Returns all asset addresses.
pub(crate) fn get_pools_list(env: &Env) -> Vec<Address> {
    env.storage()
        .persistent()
        .get(&ControllerKey::PoolsList)
        .unwrap_or_else(|| Vec::new(env))
}

// Bumps PoolsList TTL.
pub(crate) fn renew_pools_list(env: &Env) {
    let key = ControllerKey::PoolsList;
    if env.storage().persistent().has(&key) {
        renew_protocol_shared_key(env, &key);
    }
}

// Adds asset to PoolsList. Idempotent and capped at MAX_POOLS_LIST_ENTRIES.
pub(crate) fn add_to_pools_list(env: &Env, asset: &Address) {
    let mut list = get_pools_list(env);
    if list.iter().any(|existing| &existing == asset) {
        return;
    }
    assert_with_error!(
        env,
        list.len() < MAX_POOLS_LIST_ENTRIES,
        GenericError::InvalidPositionLimits
    );
    list.push_back(asset.clone());
    let key = ControllerKey::PoolsList;
    env.storage().persistent().set(&key, &list);
    renew_protocol_shared_key(env, &key);
}
