use super::bump_shared;
use common::errors::GenericError;
use common::types::{ControllerKey, MarketConfig};
use soroban_sdk::{panic_with_error, Address, Env};

pub fn get_market_config(env: &Env, asset: &Address) -> MarketConfig {
    let key = ControllerKey::Market(asset.clone());
    match env.storage().persistent().get::<_, MarketConfig>(&key) {
        Some(config) => config,
        None => panic_with_error!(env, GenericError::AssetNotSupported),
    }
}

pub fn set_market_config(env: &Env, asset: &Address, config: &MarketConfig) {
    let key = ControllerKey::Market(asset.clone());
    env.storage().persistent().set(&key, config);
    bump_shared(env, &key);
}

pub fn has_market_config(env: &Env, asset: &Address) -> bool {
    let key = ControllerKey::Market(asset.clone());
    env.storage().persistent().has(&key)
}

pub fn try_get_market_config(env: &Env, asset: &Address) -> Option<MarketConfig> {
    let key = ControllerKey::Market(asset.clone());
    env.storage().persistent().get(&key)
}
