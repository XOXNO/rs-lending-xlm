//! Persistent token-rooted oracle config storage.
//!
//! `AssetOracle(asset)` holds each asset's resolved `AssetOracleConfig` in the
//! protocol-shared persistent tier, TTL-renewed on every read and write.

use common::constants::{TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};
use common::types::AssetOracleConfig;
use soroban_sdk::{contracttype, Address, Env};

#[contracttype]
enum AggregatorKey {
    AssetOracle(Address),
}

/// Token-rooted oracle config for `asset`, renewing its shared-tier TTL on hit.
pub(crate) fn get_oracle_config(env: &Env, asset: &Address) -> Option<AssetOracleConfig> {
    let key = AggregatorKey::AssetOracle(asset.clone());
    let config: Option<AssetOracleConfig> = env.storage().persistent().get(&key);
    if config.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
    }
    config
}

/// Stores the token-rooted oracle config and renews its shared-tier TTL.
pub(crate) fn set_oracle_config(env: &Env, asset: &Address, config: &AssetOracleConfig) {
    let key = AggregatorKey::AssetOracle(asset.clone());
    env.storage().persistent().set(&key, config);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}

/// Test-only: removes an asset's oracle (disabling pricing for it).
#[cfg(any(test, feature = "testing"))]
pub(crate) fn remove_oracle_config(env: &Env, asset: &Address) {
    env.storage()
        .persistent()
        .remove(&AggregatorKey::AssetOracle(asset.clone()));
}
