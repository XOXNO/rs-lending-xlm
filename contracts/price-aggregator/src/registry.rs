//! Persistent storage of oracle configurations: keyed lookup and storage with TTL
//! extension, the registered-keys index, and the event emitted on configuration
//! changes.

use common::constants::{TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};
use common::types::{AssetOracle, PriceKey};
use soroban_sdk::{contractevent, contracttype, Env, Vec};

/// Instance/persistent storage keys used by this contract: a single asset
/// oracle's configuration, or the index of all registered oracle keys.
#[contracttype]
enum AggregatorKey {
    Oracle(PriceKey),
    OracleKeys,
}

/// Returns the list of all currently registered oracle keys, or an empty list
/// if none have been registered.
pub(crate) fn oracle_keys(env: &Env) -> Vec<PriceKey> {
    env.storage()
        .instance()
        .get(&AggregatorKey::OracleKeys)
        .unwrap_or_else(|| Vec::new(env))
}

/// Overwrites the registered-oracle-keys index with `keys`.
fn store_keys(env: &Env, keys: &Vec<PriceKey>) {
    env.storage()
        .instance()
        .set(&AggregatorKey::OracleKeys, keys);
}

/// Returns the oracle configuration stored for `key`, if any, extending its
/// persistent-storage TTL when found.
pub(crate) fn get_oracle(env: &Env, key: &PriceKey) -> Option<AssetOracle> {
    let storage_key = AggregatorKey::Oracle(key.clone());
    let oracle = env.storage().persistent().get(&storage_key);
    if oracle.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&storage_key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
    }
    oracle
}

/// Stores `oracle` under `key`, extending its persistent-storage TTL, and adds
/// `key` to the registered-keys index if it is not already present.
pub(crate) fn store_oracle(env: &Env, key: &PriceKey, oracle: &AssetOracle) {
    let storage_key = AggregatorKey::Oracle(key.clone());
    env.storage().persistent().set(&storage_key, oracle);
    env.storage()
        .persistent()
        .extend_ttl(&storage_key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);

    let mut registered = oracle_keys(env);
    if !registered.contains(key) {
        registered.push_back(key.clone());
        store_keys(env, &registered);
    }
}

/// Removes the oracle configuration stored for `key` and drops `key` from the
/// registered-keys index if present.
#[cfg(any(test, feature = "testing"))]
pub(crate) fn remove_oracle(env: &Env, key: &PriceKey) {
    env.storage()
        .persistent()
        .remove(&AggregatorKey::Oracle(key.clone()));
    let mut registered = oracle_keys(env);
    if let Some(index) = registered.first_index_of(key) {
        registered.remove(index);
        store_keys(env, &registered);
    }
}

/// Stores `oracle` under `key` and emits the corresponding update event.
pub(crate) fn commit(env: &Env, key: &PriceKey, oracle: &AssetOracle) {
    store_oracle(env, key, oracle);
    emit(env, key, oracle);
}

/// Publishes an `UpdateAssetOracleEvent` for `key` and `oracle`.
pub(crate) fn emit(env: &Env, key: &PriceKey, oracle: &AssetOracle) {
    UpdateAssetOracleEvent {
        key: key.clone(),
        oracle: oracle.clone(),
    }
    .publish(env);
}

/// Event emitted whenever an asset's oracle configuration is stored or
/// updated, carrying the affected key and the new configuration.
#[contractevent(topics = ["config", "asset_oracle"])]
#[derive(Clone, Debug)]
pub struct UpdateAssetOracleEvent {
    pub key: PriceKey,
    pub oracle: AssetOracle,
}

#[cfg(test)]
#[path = "../tests/oracle/registry.rs"]
mod tests;
