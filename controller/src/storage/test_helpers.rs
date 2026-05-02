use super::{get_market_config, set_market_config};
use common::types::{OracleProviderConfig, ReflectorConfig};
use soroban_sdk::{Address, Env};

pub fn set_reflector_config(env: &Env, asset: &Address, config: &ReflectorConfig) {
    let mut market = get_market_config(env, asset);
    market.cex_oracle = Some(config.cex_oracle.clone());
    market.cex_asset_kind = config.cex_asset_kind.clone();
    market.cex_symbol = config.cex_symbol.clone();
    market.cex_decimals = config.cex_decimals;
    market.dex_oracle = config.dex_oracle.clone();
    market.dex_asset_kind = config.dex_asset_kind.clone();
    market.dex_decimals = config.dex_decimals;
    market.twap_records = config.twap_records;
    set_market_config(env, asset, &market);
}

pub fn set_oracle_config(env: &Env, asset: &Address, config: &OracleProviderConfig) {
    let mut market = get_market_config(env, asset);
    market.oracle_config = config.clone();
    set_market_config(env, asset, &market);
}
