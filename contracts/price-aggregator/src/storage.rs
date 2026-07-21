//! Persistent token-rooted oracle config storage.
//!
//! `AssetOracle(asset)` holds each asset's resolved `MarketOracleConfig` in the
//! protocol-shared persistent tier, TTL-renewed on every read and write.

use common::constants::{TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};
use common::types::MarketOracleConfig;
use soroban_sdk::{contracttype, Address, Env};

#[contracttype]
enum AggregatorKey {
    AssetOracle(Address),
}

/// Token-rooted oracle config for `asset`, renewing its shared-tier TTL on hit.
pub(crate) fn get_asset_oracle(env: &Env, asset: &Address) -> Option<MarketOracleConfig> {
    let key = AggregatorKey::AssetOracle(asset.clone());
    let config: Option<MarketOracleConfig> = env.storage().persistent().get(&key);
    if config.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
    }
    config
}

/// Stores the token-rooted oracle config and renews its shared-tier TTL.
pub(crate) fn set_asset_oracle(env: &Env, asset: &Address, config: &MarketOracleConfig) {
    let key = AggregatorKey::AssetOracle(asset.clone());
    env.storage().persistent().set(&key, config);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}
