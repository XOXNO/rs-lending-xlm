use common::errors::{GenericError, OracleError};
use common::types::{
    MarketOracleConfig, MarketOracleConfigInput, OraclePriceFluctuation, OracleReadMode,
    OracleSourceConfig, OracleSourceConfigInput, OracleStrategy, RedStoneSourceConfig,
    ReflectorSourceConfig,
};
use soroban_sdk::{panic_with_error, token, Address, Env};

use super::observation::{
    millis_to_seconds, normalize_positive_price, u256_to_i128, validate_timestamp,
    MAX_ORACLE_DECIMALS, MAX_PRICE_STALE_SECONDS, MAX_TWAP_RECORDS, MIN_ORACLE_DECIMALS,
    MIN_PRICE_STALE_SECONDS,
};
use super::providers::redstone::{RedStonePriceData, RedStonePriceFeedClient, REDSTONE_DECIMALS};
use super::providers::reflector::{min_twap_observations, to_reflector_asset};
use super::reflector::{
    reflector_base_call, reflector_lastprice_call, reflector_prices_call, ReflectorAsset,
    ReflectorClient, ReflectorPriceData,
};

pub(crate) fn validate_market_oracle_sources(
    env: &Env,
    asset: &Address,
    config: &MarketOracleConfigInput,
    tolerance: OraclePriceFluctuation,
) -> MarketOracleConfig {
    validate_oracle_config_shape(env, config);
    validate_max_stale(env, config.max_price_stale_seconds);
    validate_sanity_bounds(env, config.min_sanity_price_wad, config.max_sanity_price_wad);

    let asset_decimals = validate_oracle_asset(env, asset);
    let primary = validate_source(env, &config.primary, config.max_price_stale_seconds);
    let anchor = match config.anchor.as_ref() {
        Some(anchor) => common::types::OracleSourceConfigOption::Some(validate_source(
            env,
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
        min_sanity_price_wad: config.min_sanity_price_wad,
        max_sanity_price_wad: config.max_sanity_price_wad,
    }
}


fn validate_oracle_config_shape(env: &Env, config: &MarketOracleConfigInput) {
    let needs_anchor = config.strategy == OracleStrategy::PrimaryWithAnchor;
    let has_anchor = !config.anchor.is_none();
    if needs_anchor != has_anchor {
        panic_with_error!(env, GenericError::InvalidExchangeSrc);
    }
}

fn validate_oracle_asset(env: &Env, asset: &Address) -> u32 {
    let token_decimals = unwrap_token_call(
        env,
        token::Client::new(env, asset).try_decimals().map(|r| r.ok()),
    );
    if token::Client::new(env, asset).try_symbol().is_err() {
        panic_with_error!(env, GenericError::InvalidAsset);
    }
    token_decimals
}


fn unwrap_token_call<T>(
    env: &Env,
    result: Result<Option<T>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
) -> T {
    match result {
        Ok(Some(value)) => value,
        _ => panic_with_error!(env, GenericError::InvalidAsset),
    }
}

fn validate_source(
    env: &Env,
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
    if !(MIN_PRICE_STALE_SECONDS..=MAX_PRICE_STALE_SECONDS).contains(&max_stale) {
        panic_with_error!(env, OracleError::InvalidStalenessConfig);
    }
}

// Validate sanity bounds.
fn validate_sanity_bounds(env: &Env, min_wad: i128, max_wad: i128) {
    if min_wad <= 0 || max_wad <= 0 || min_wad >= max_wad {
        panic_with_error!(env, OracleError::InvalidSanityBounds);
    }
}

fn validate_usd_base(env: &Env, oracle: &Address) {
    match reflector_base_call(env, oracle) {
        ReflectorAsset::Other(symbol) if symbol == soroban_sdk::Symbol::new(env, "USD") => {}
        _ => panic_with_error!(env, OracleError::InvalidOracleBase),
    }
}

fn validate_decimals(env: &Env, decimals: u32) {
    if !(MIN_ORACLE_DECIMALS..=MAX_ORACLE_DECIMALS).contains(&decimals) {
        panic_with_error!(env, OracleError::InvalidOracleDecimals);
    }
}

fn validate_twap_records(env: &Env, records: u32) {
    if records == 0 {
        panic_with_error!(env, OracleError::TwapInsufficientObservations);
    }
    if records > MAX_TWAP_RECORDS {
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
