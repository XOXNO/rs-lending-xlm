//! Live oracle validation for market configuration.

use common::errors::{GenericError, OracleError};
use common::oracle::observation::{
    millis_to_seconds, u256_to_i128, validate_positive_price_timestamps,
    MIN_ORACLE_RESOLUTION_SECONDS,
};
use common::oracle::providers::redstone::{
    read_price_data_uncached, RedStonePriceData, REDSTONE_DECIMALS,
};
use common::oracle::providers::reflector::{
    min_twap_observations, reflector_base_call, reflector_decimals_call, reflector_lastprice_call,
    reflector_prices_call, reflector_resolution_call, to_reflector_asset, ReflectorAsset,
    ReflectorPriceData,
};
use common::types::{
    MarketOracleConfig, MarketOracleConfigInput, OraclePriceFluctuation, OracleReadMode,
    OracleSourceConfig, OracleSourceConfigInput, OracleSourceConfigOption, RedStoneSourceConfig,
    RedStoneSourceConfigInput, ReflectorBase, ReflectorSourceConfig,
};
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env};

use crate::validate::asset::validate_and_fetch_token_decimals;
use crate::validate::oracle_config::{
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
    common::validation::validate_single_source_sanity_band(
        env,
        config.strategy,
        config.min_sanity_price_wad,
        config.max_sanity_price_wad,
    );

    let asset_decimals = validate_and_fetch_token_decimals(env, asset);
    let primary = validate_source(env, asset, &config.primary, config.max_price_stale_seconds);
    let anchor = match config.anchor.as_ref() {
        Some(anchor) => OracleSourceConfigOption::Some(validate_source(
            env,
            asset,
            anchor,
            config.max_price_stale_seconds,
        )),
        None => OracleSourceConfigOption::None,
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

fn validate_source(
    env: &Env,
    asset: &Address,
    source: &OracleSourceConfigInput,
    max_stale: u64,
) -> OracleSourceConfig {
    match source {
        OracleSourceConfigInput::Reflector(config) => {
            let base = validate_base(env, asset, &config.contract);
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
                base,
            })
        }
        OracleSourceConfigInput::RedStone(config) => {
            // RedStone feeds are 8-decimal; the adapter has no `decimals()`.
            let redstone = validate_feed_id_source(env, config, REDSTONE_DECIMALS);
            OracleSourceConfig::RedStone(redstone)
        }
        OracleSourceConfigInput::Xoxno(config) => {
            let decimals = reflector_decimals_call(env, &config.contract);
            let xoxno = validate_feed_id_source(env, config, decimals);
            OracleSourceConfig::Xoxno(xoxno)
        }
    }
}

/// Validates a RedStone-shaped (feed-id keyed) source with a live feed probe.
fn validate_feed_id_source(
    env: &Env,
    config: &RedStoneSourceConfigInput,
    decimals: u32,
) -> RedStoneSourceConfig {
    validate_max_stale(env, config.max_stale_seconds);
    validate_decimals(env, decimals);

    let Some(price_data) = read_price_data_uncached(env, &config.contract, &config.feed_id) else {
        panic_with_error!(env, GenericError::InvalidTicker);
    };
    validate_redstone_feed(env, &price_data, config.max_stale_seconds, decimals);

    RedStoneSourceConfig {
        contract: config.contract.clone(),
        feed_id: config.feed_id.clone(),
        decimals,
        max_stale_seconds: config.max_stale_seconds,
    }
}

/// Resolves Reflector base; controller re-checks quote activation.
fn validate_base(env: &Env, asset: &Address, oracle: &Address) -> ReflectorBase {
    match reflector_base_call(env, oracle) {
        ReflectorAsset::Other(symbol) if symbol == soroban_sdk::Symbol::new(env, "USD") => {
            ReflectorBase::Usd
        }
        ReflectorAsset::Stellar(quote) => {
            // Reject self-quotes to avoid recursive price reads.
            assert_with_error!(env, &quote != asset, OracleError::InvalidOracleBase);
            ReflectorBase::Quoted(quote)
        }
        ReflectorAsset::Other(_) => panic_with_error!(env, OracleError::InvalidOracleBase),
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
    let now = env.ledger().timestamp();
    let _ = validate_positive_price_timestamps(
        env,
        pd.price,
        decimals,
        now,
        &[pd.timestamp],
        max_stale,
    );
}

fn validate_redstone_feed(env: &Env, pd: &RedStonePriceData, max_stale: u64, decimals: u32) {
    let raw_price = u256_to_i128(env, &pd.price);
    let now = env.ledger().timestamp();
    let _ = validate_positive_price_timestamps(
        env,
        raw_price,
        decimals,
        now,
        &[
            millis_to_seconds(pd.package_timestamp),
            millis_to_seconds(pd.write_timestamp),
        ],
        max_stale,
    );
}
