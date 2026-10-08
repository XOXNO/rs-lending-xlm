//! Persistent storage of oracle configurations: keyed lookup and storage with TTL
//! extension, the registered-keys index, the reverse-dependency index, and the
//! event emitted on configuration changes.

use common::constants::{TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};
use common::types::{AssetOracle, PriceKey};
use soroban_sdk::{contractevent, contracttype, Env, Vec};

use crate::properties;

/// Storage keys: `Oracle` (persistent) holds one asset oracle's configuration,
/// `OracleKeys` (instance) indexes all registered oracle keys, `Dependents`
/// (persistent) lists the registered keys whose sources read a key directly,
/// and `DependentsIndexed` (instance) marks that `Dependents` covers every
/// registered oracle.
#[contracttype]
enum AggregatorKey {
    Oracle(PriceKey),
    OracleKeys,
    Dependents(PriceKey),
    DependentsIndexed,
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

/// Stores `oracle` under `key`, extending its persistent-storage TTL, moves
/// `key` between the dependents lists of the keys it stops and starts reading,
/// and adds `key` to the registered-keys index if it is not already present.
pub(crate) fn store_oracle(env: &Env, key: &PriceKey, oracle: &AssetOracle) {
    let storage_key = AggregatorKey::Oracle(key.clone());
    let previous: Option<AssetOracle> = env.storage().persistent().get(&storage_key);
    relink(env, key, previous.as_ref(), Some(oracle));
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

/// Removes the oracle configuration stored for `key`, drops `key` from the
/// dependents lists of the keys it read, and drops it from the
/// registered-keys index if present.
#[cfg(any(test, feature = "testing"))]
pub(crate) fn remove_oracle(env: &Env, key: &PriceKey) {
    let storage_key = AggregatorKey::Oracle(key.clone());
    let previous: Option<AssetOracle> = env.storage().persistent().get(&storage_key);
    relink(env, key, previous.as_ref(), None);
    env.storage().persistent().remove(&storage_key);
    let mut registered = oracle_keys(env);
    if let Some(index) = registered.first_index_of(key) {
        registered.remove(index);
        store_keys(env, &registered);
    }
}

/// Returns the registered keys whose sources read `key` directly, extending
/// the list's persistent-storage TTL when one is stored.
pub(crate) fn dependents(env: &Env, key: &PriceKey) -> Vec<PriceKey> {
    let storage_key = AggregatorKey::Dependents(key.clone());
    let Some(dependents) = env.storage().persistent().get(&storage_key) else {
        return Vec::new(env);
    };
    env.storage()
        .persistent()
        .extend_ttl(&storage_key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
    dependents
}

/// Indexes every registered oracle's dependencies once, for a registry stored
/// before the dependents index existed. Every later write keeps the index
/// current through `store_oracle`, so after the first call this reads only the
/// instance flag.
pub(crate) fn ensure_dependents_indexed(env: &Env) {
    if env
        .storage()
        .instance()
        .has(&AggregatorKey::DependentsIndexed)
    {
        return;
    }
    for key in oracle_keys(env).iter() {
        if let Some(oracle) = get_oracle(env, &key) {
            relink(env, &key, None, Some(&oracle));
        }
    }
    env.storage()
        .instance()
        .set(&AggregatorKey::DependentsIndexed, &true);
}

/// Removes `key` from the dependents list of every key `previous` read and
/// `next` does not, and adds it to the list of every key `next` reads.
/// Adding is idempotent, so relinking an already indexed oracle writes
/// nothing.
fn relink(env: &Env, key: &PriceKey, previous: Option<&AssetOracle>, next: Option<&AssetOracle>) {
    let before = oracle_dependencies(env, previous);
    let after = oracle_dependencies(env, next);
    for dependency in before.iter() {
        if !after.contains(&dependency) {
            let mut list = dependents(env, &dependency);
            if let Some(index) = list.first_index_of(key) {
                list.remove(index);
                store_dependents(env, &dependency, &list);
            }
        }
    }
    for dependency in after.iter() {
        let mut list = dependents(env, &dependency);
        if !list.contains(key) {
            list.push_back(key.clone());
            store_dependents(env, &dependency, &list);
        }
    }
}

/// Returns the distinct keys `oracle`'s sources read directly, or none when
/// there is no oracle.
fn oracle_dependencies(env: &Env, oracle: Option<&AssetOracle>) -> Vec<PriceKey> {
    let mut keys = Vec::new(env);
    for source in oracle.iter().flat_map(|oracle| oracle.sources.iter()) {
        for dependency in properties::dependencies(env, &source).iter() {
            if !keys.contains(&dependency) {
                keys.push_back(dependency);
            }
        }
    }
    keys
}

/// Overwrites the dependents list of `key`, removing the entry when the list
/// is empty.
fn store_dependents(env: &Env, key: &PriceKey, list: &Vec<PriceKey>) {
    let storage_key = AggregatorKey::Dependents(key.clone());
    if list.is_empty() {
        env.storage().persistent().remove(&storage_key);
        return;
    }
    env.storage().persistent().set(&storage_key, list);
    env.storage()
        .persistent()
        .extend_ttl(&storage_key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
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
