//! Live probing and validation against external oracle contracts.
//!
//! This module contains all logic that makes cross-contract calls to
//! Reflector or RedStone oracles during market configuration and TWAP
//! history validation.

use common::errors::{GenericError, OracleError};
use common::types::{
    MarketOracleConfig, MarketOracleConfigInput, OraclePriceFluctuation, OracleReadMode,
    OracleSourceConfig, OracleSourceConfigInput, RedStoneSourceConfig, ReflectorSourceConfig,
};
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env};

use crate::validation;

use super::super::observation::{
    millis_to_seconds, normalize_positive_price, u256_to_i128, validate_timestamp,
    MIN_ORACLE_RESOLUTION_SECONDS,
};
use super::super::providers::redstone::{read_price_data, RedStonePriceData, REDSTONE_DECIMALS};
use super::super::providers::reflector::{
    min_twap_observations, reflector_base_call, reflector_decimals_call, reflector_lastprice_call,
    reflector_prices_call, reflector_resolution_call, to_reflector_asset, ReflectorAsset,
    ReflectorPriceData,
};
use super::config::{
    validate_decimals, validate_max_stale, validate_oracle_config_shape, validate_sanity_bounds,
    validate_twap_records,
};

pub(crate) fn validate_market_oracle_sources(
    env: &Env,
    asset: &Address,
    config: &MarketOracleConfigInput,
    tolerance: OraclePriceFluctuation,
) -> MarketOracleConfig {
    validate_oracle_config_shape(env, config);
    validate_max_stale(env, config.max_price_stale_seconds);
    validate_sanity_bounds(
        env,
        config.min_sanity_price_wad,
        config.max_sanity_price_wad,
    );

    let asset_decimals = validation::validate_and_fetch_token_decimals(env, asset);
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

pub(crate) fn validate_source(
    env: &Env,
    source: &OracleSourceConfigInput,
    max_stale: u64,
) -> OracleSourceConfig {
    match source {
        OracleSourceConfigInput::Reflector(config) => {
            validate_usd_base(env, &config.contract);
            let reflector_asset = to_reflector_asset(env, &config.asset);
            let decimals = reflector_decimals_call(env, &config.contract);
            validate_decimals(env, decimals);
            let resolution = reflector_resolution_call(env, &config.contract);
            if resolution < MIN_ORACLE_RESOLUTION_SECONDS || u64::from(resolution) > max_stale {
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
                read_mode: config.read_mode,
                decimals,
                resolution_seconds: resolution,
            })
        }
        OracleSourceConfigInput::RedStone(config) => {
            validate_max_stale(env, config.max_stale_seconds);

            // Redstone has no on-chain base() accessor; quote currency is
            // implicit in `feed_id`. See providers/redstone.rs for the full
            // identity-validation note.
            let decimals = REDSTONE_DECIMALS;
            validate_decimals(env, decimals);

            let price_data = match read_price_data(env, &config.contract, &config.feed_id) {
                Some(data) => data,
                _ => panic_with_error!(env, GenericError::InvalidTicker),
            };
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

fn validate_usd_base(env: &Env, oracle: &Address) {
    match reflector_base_call(env, oracle) {
        ReflectorAsset::Other(symbol) if symbol == soroban_sdk::Symbol::new(env, "USD") => {}
        _ => panic_with_error!(env, OracleError::InvalidOracleBase),
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
    assert_with_error!(env, !history.is_empty(), OracleError::ReflectorHistoryEmpty);
    assert_with_error!(
        env,
        history.len() >= min_twap_observations(records),
        OracleError::TwapInsufficientObservations
    );
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
    let now = env.ledger().timestamp();
    validate_timestamp(
        env,
        now,
        millis_to_seconds(pd.package_timestamp),
        max_stale,
    );
    validate_timestamp(
        env,
        now,
        millis_to_seconds(pd.write_timestamp),
        max_stale,
    );
}
