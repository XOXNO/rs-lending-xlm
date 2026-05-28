//! Pool registry and the canonical list of listed assets.
//!
//! The list is used off-chain by the keeper service to enumerate pools for
//! `update_indexes` sweeps and to discover storage keys whose TTL needs
//! extending. It is append-only from the perspective of normal operation
//! (removals are not supported after listing).

use super::renew_protocol_shared_key;
use common::constants::MAX_POOLS_LIST_ENTRIES;
use common::errors::GenericError;
use common::types::ControllerKey;
use soroban_sdk::{assert_with_error, Address, Env, Vec};

/// Returns listed asset addresses from the controller pool registry.
pub(crate) fn get_pools_list(env: &Env) -> Vec<Address> {
    env.storage()
        .persistent()
        .get(&ControllerKey::PoolsList)
        .unwrap_or_else(|| Vec::new(env))
}

/// Adds an asset to `PoolsList`; duplicate inserts are no-ops.
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
