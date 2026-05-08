use common::errors::{GenericError, OracleError};
use common::types::{
    MarketOracleConfig, MarketOracleConfigInput, OraclePriceFluctuation, OracleReadMode,
    OracleSourceConfig, OracleSourceConfigInput, OracleStrategy, RedStoneSourceConfig,
    ReflectorSourceConfig,
};
use soroban_sdk::{panic_with_error, token, Address, Env, U256};

use super::observation::{normalize_positive_price, validate_timestamp};
use super::providers::redstone::{RedStonePriceData, RedStonePriceFeedClient, REDSTONE_DECIMALS};
use super::providers::reflector::{min_twap_observations, to_reflector_asset};
use super::reflector::{
    reflector_base_call, reflector_lastprice_call, reflector_prices_call, ReflectorAsset,
    ReflectorClient, ReflectorPriceData,
};

const MAX_ORACLE_DECIMALS: u32 = 18;

pub(crate) fn validate_market_oracle_sources(
    env: &Env,
    asset: &Address,
    config: &MarketOracleConfigInput,
    tolerance: OraclePriceFluctuation,
) -> MarketOracleConfig {
    validate_oracle_config_shape(env, config);

    let asset_decimals = validate_oracle_asset(env, asset);
    let primary = validate_source(env, asset, &config.primary, config.max_price_stale_seconds);
    let anchor = match config.anchor.as_ref() {
        Some(anchor) => common::types::OracleSourceConfigOption::Some(validate_source(
            env,
            asset,
            anchor,
            config.max_price_stale_seconds,
        )),
        None => common::types::OracleSourceConfigOption::None,
    };

    MarketOracleConfig {
        asset_decimals,
        max_price_stale_seconds: config.max_price_stale_seconds,
        tolerance,
        strategy: config.strategy,
        primary,
        anchor,
    }
}

fn validate_oracle_config_shape(env: &Env, config: &MarketOracleConfigInput) {
    if config.strategy == OracleStrategy::PrimaryWithAnchor && config.anchor.is_none() {
        panic_with_error!(env, GenericError::InvalidExchangeSrc);
    }
    if config.strategy == OracleStrategy::Single && !config.anchor.is_none() {
        panic_with_error!(env, GenericError::InvalidExchangeSrc);
    }
}

fn validate_oracle_asset(env: &Env, asset: &Address) -> u32 {
    let token_decimals = token::Client::new(env, asset)
        .try_decimals()
        .unwrap_or_else(|_| panic_with_error!(env, GenericError::InvalidAsset))
        .unwrap_or_else(|_| panic_with_error!(env, GenericError::InvalidAsset));
    if token::Client::new(env, asset).try_symbol().is_err() {
        panic_with_error!(env, GenericError::InvalidAsset);
    }
    token_decimals
}

fn validate_source(
    env: &Env,
    _asset: &Address,
    source: &OracleSourceConfigInput,
    max_stale: u64,
) -> OracleSourceConfig {
    match source {
        OracleSourceConfigInput::Reflector(config) => {
            validate_usd_base(env, &config.contract);
            let reflector_asset = to_reflector_asset(env, &config.asset);
            let client = ReflectorClient::new(env, &config.contract);
            let decimals = client.decimals();
            validate_decimals(env, decimals);
            let resolution = client.resolution();
            if resolution == 0 || u64::from(resolution) > max_stale {
                panic_with_error!(env, OracleError::InvalidOracleResolution);
            }

            let pd = reflector_lastprice_call(env, &config.contract, &reflector_asset)
                .unwrap_or_else(|| panic_with_error!(env, GenericError::InvalidTicker));
            validate_reflector_feed(env, &pd, max_stale, decimals);

            match config.read_mode {
                OracleReadMode::Spot => {}
                OracleReadMode::Twap(records) => {
                    validate_twap_records(env, records);
                    validate_twap_history(
                        env,
                        &config.contract,
                        &reflector_asset,
                        records,
                        max_stale,
                        decimals,
                    );
                }
            }

            OracleSourceConfig::Reflector(ReflectorSourceConfig {
                contract: config.contract.clone(),
                asset: config.asset.clone(),
                read_mode: config.read_mode.clone(),
                decimals,
                resolution_seconds: resolution,
            })
        }
        OracleSourceConfigInput::RedStone(config) => {
            validate_max_stale(env, config.max_stale_seconds);

            let client = RedStonePriceFeedClient::new(env, &config.contract);
            let decimals = REDSTONE_DECIMALS;
            validate_decimals(env, decimals);

            let price_data = client
                .try_read_price_data_for_feed(&config.feed_id)
                .unwrap_or_else(|_| panic_with_error!(env, GenericError::InvalidTicker))
                .unwrap_or_else(|_| panic_with_error!(env, GenericError::InvalidTicker));
            validate_redstone_feed(env, &price_data, config.max_stale_seconds, decimals);

            OracleSourceConfig::RedStone(RedStoneSourceConfig {
                contract: config.contract.clone(),
                feed_id: config.feed_id.clone(),
                decimals,
                max_stale_seconds: config.max_stale_seconds,
            })
        }
    }
}

fn validate_max_stale(env: &Env, max_stale: u64) {
    if max_stale < 60 || max_stale > 86_400 {
        panic_with_error!(env, OracleError::InvalidStalenessConfig);
    }
}

fn validate_usd_base(env: &Env, oracle: &Address) {
    match reflector_base_call(env, oracle) {
        ReflectorAsset::Other(symbol) if symbol == soroban_sdk::Symbol::new(env, "USD") => {}
        _ => panic_with_error!(env, OracleError::InvalidOracleBase),
    }
}

fn validate_decimals(env: &Env, decimals: u32) {
    if decimals > MAX_ORACLE_DECIMALS {
        panic_with_error!(env, OracleError::InvalidOracleDecimals);
    }
}

fn validate_twap_records(env: &Env, records: u32) {
    if records == 0 {
        panic_with_error!(env, OracleError::TwapInsufficientObservations);
    }
    if records > 12 {
        panic_with_error!(env, OracleError::InvalidOracleTokenType);
    }
}

fn validate_twap_history(
    env: &Env,
    oracle: &Address,
    asset: &ReflectorAsset,
    records: u32,
    max_stale: u64,
    decimals: u32,
) {
    let history = reflector_prices_call(env, oracle, asset, records)
        .unwrap_or_else(|| panic_with_error!(env, OracleError::ReflectorHistoryEmpty));
    if history.is_empty() {
        panic_with_error!(env, OracleError::ReflectorHistoryEmpty);
    }
    if history.len() < min_twap_observations(records) {
        panic_with_error!(env, OracleError::TwapInsufficientObservations);
    }
    for pd in history.iter() {
        validate_reflector_feed(env, &pd, max_stale, decimals);
    }
}

fn validate_reflector_feed(env: &Env, pd: &ReflectorPriceData, max_stale: u64, decimals: u32) {
    let _ = normalize_positive_price(env, pd.price, decimals);
    validate_timestamp(env, env.ledger().timestamp(), pd.timestamp, max_stale);
}

fn validate_redstone_feed(env: &Env, pd: &RedStonePriceData, max_stale: u64, decimals: u32) {
    let raw_price = u256_to_i128(env, &pd.price);
    let _ = normalize_positive_price(env, raw_price, decimals);
    validate_timestamp(
        env,
        env.ledger().timestamp(),
        millis_to_seconds(env, pd.package_timestamp),
        max_stale,
    );
    validate_timestamp(
        env,
        env.ledger().timestamp(),
        millis_to_seconds(env, pd.write_timestamp),
        max_stale,
    );
}

fn u256_to_i128(env: &Env, value: &U256) -> i128 {
    let Some(raw) = value.to_u128() else {
        panic_with_error!(env, GenericError::MathOverflow);
    };
    if raw > i128::MAX as u128 {
        panic_with_error!(env, GenericError::MathOverflow);
    }
    raw as i128
}

fn millis_to_seconds(_env: &Env, timestamp_ms: u64) -> u64 {
    timestamp_ms / 1000
}
