use common::constants::{TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};
use common::types::{AssetOracle, PriceKey};
use soroban_sdk::{contractevent, contracttype, Env, Vec};

#[contracttype]
enum AggregatorKey {
    Oracle(PriceKey),
    OracleKeys,
}

pub(crate) fn oracle_keys(env: &Env) -> Vec<PriceKey> {
    env.storage()
        .instance()
        .get(&AggregatorKey::OracleKeys)
        .unwrap_or_else(|| Vec::new(env))
}

fn store_keys(env: &Env, keys: &Vec<PriceKey>) {
    env.storage()
        .instance()
        .set(&AggregatorKey::OracleKeys, keys);
}

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

pub(crate) fn commit(env: &Env, key: &PriceKey, oracle: &AssetOracle) {
    store_oracle(env, key, oracle);
    emit(env, key, oracle);
}

pub(crate) fn emit(env: &Env, key: &PriceKey, oracle: &AssetOracle) {
    UpdateAssetOracleEvent {
        key: key.clone(),
        oracle: oracle.clone(),
    }
    .publish(env);
}

#[contractevent(topics = ["config", "asset_oracle"])]
#[derive(Clone, Debug)]
pub struct UpdateAssetOracleEvent {
    pub key: PriceKey,
    pub oracle: AssetOracle,
}

#[cfg(test)]
#[path = "../tests/oracle/registry.rs"]
mod tests;
