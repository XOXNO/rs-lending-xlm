//! Market configuration storage.
//!
//! `MarketConfig` (status, pool address, full `AssetConfig`, and the
//! complete `MarketOracleConfig`) is the source of truth for every risk
//! and oracle decision. It lives under a protocol-shared key and therefore
//! receives the shared TTL policy via `renew_protocol_shared_key`.

use super::renew_protocol_shared_key;
use common::errors::GenericError;
use common::types::{ControllerKey, MarketConfig};
use soroban_sdk::{panic_with_error, Address, Env};

pub(crate) fn get_market_config(env: &Env, asset: &Address) -> MarketConfig {
    try_get_market_config(env, asset)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AssetNotSupported))
}

pub(crate) fn set_market_config(env: &Env, asset: &Address, config: &MarketConfig) {
    let key = ControllerKey::Market(asset.clone());
    env.storage().persistent().set(&key, config);
    renew_protocol_shared_key(env, &key);
}

pub(crate) fn has_market_config(env: &Env, asset: &Address) -> bool {
    let key = ControllerKey::Market(asset.clone());
    env.storage().persistent().has(&key)
}

pub(crate) fn try_get_market_config(env: &Env, asset: &Address) -> Option<MarketConfig> {
    let key = ControllerKey::Market(asset.clone());
    env.storage().persistent().get(&key)
}
