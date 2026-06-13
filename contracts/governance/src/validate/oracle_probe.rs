//! Live probing and validation against external oracle contracts.
//!
//! This module contains all logic that makes cross-contract calls to
//! Reflector or RedStone oracles during market configuration and TWAP
//! history validation. Quote-market eligibility reads back through the
//! controller's `get_market_config` view.

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
use controller_interface::types::{
    MarketOracleConfig, MarketOracleConfigInput, MarketStatus, OraclePriceFluctuation,
    OracleReadMode, OracleSourceConfig, RedStoneSourceConfig, ReflectorBase, ReflectorSourceConfig,
};
use controller_interface::ControllerAdminClient;
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env};

use super::asset::validate_and_fetch_token_decimals;
use super::oracle_config::{
    validate_decimals, validate_max_stale, validate_oracle_config_shape, validate_sanity_bounds,
    validate_twap_records,
};

pub(crate) fn validate_market_oracle_sources(
    env: &Env,
    controller: &Address,
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

    let asset_decimals = validate_and_fetch_token_decimals(env, asset);
    let primary = validate_source(
        env,
        controller,
        asset,
        &config.primary,
        config.max_price_stale_seconds,
    );
    let anchor = match config.anchor.as_ref() {
        Some(anchor) => {
            controller_interface::types::OracleSourceConfigOption::Some(validate_source(
                env,
                controller,
                asset,
                anchor,
                config.max_price_stale_seconds,
            ))
        }
        None => controller_interface::types::OracleSourceConfigOption::None,
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
    controller: &Address,
    asset: &Address,
    source: &controller_interface::types::OracleSourceConfigInput,
    max_stale: u64,
) -> OracleSourceConfig {
    match source {
        controller_interface::types::OracleSourceConfigInput::Reflector(config) => {
            let base = validate_base(env, controller, asset, &config.contract);
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
        controller_interface::types::OracleSourceConfigInput::RedStone(config) => {
            validate_max_stale(env, config.max_stale_seconds);

            // Redstone has no on-chain base() accessor; quote currency is
            // implicit in `feed_id`. See common::oracle::providers::redstone
            // for the full identity-validation note.
            let decimals = REDSTONE_DECIMALS;
            validate_decimals(env, decimals);

            let price_data = match read_price_data_uncached(env, &config.contract, &config.feed_id)
            {
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

/// Validates Reflector base currency and quote-market eligibility.
fn validate_base(
    env: &Env,
    controller: &Address,
    asset: &Address,
    oracle: &Address,
) -> ReflectorBase {
    match reflector_base_call(env, oracle) {
        ReflectorAsset::Other(symbol) if symbol == soroban_sdk::Symbol::new(env, "USD") => {
            ReflectorBase::Usd
        }
        ReflectorAsset::Stellar(quote) => {
            // A market may not be quoted in itself: the quote check below reads
            // the asset's pre-update config, so a self-quote would slip past it
            // and only revert at read time via the host recursion cap.
            assert_with_error!(env, &quote != asset, OracleError::InvalidOracleBase);
            validate_quote_is_usd_market(env, controller, &quote);
            ReflectorBase::Quoted(quote)
        }
        _ => panic_with_error!(env, OracleError::InvalidOracleBase),
    }
}

fn validate_quote_is_usd_market(env: &Env, controller: &Address, quote: &Address) {
    // The quote market lives in controller storage; missing or failing reads
    // map to the same error the missing-market case raises.
    let market = match ControllerAdminClient::new(env, controller).try_get_market_config(quote) {
        Ok(Ok(market)) => market,
        _ => panic_with_error!(env, OracleError::InvalidOracleBase),
    };
    assert_with_error!(
        env,
        market.status == MarketStatus::Active,
        OracleError::InvalidOracleBase
    );
    match &market.oracle_config.primary {
        // RedStone feeds are USD-denominated by construction.
        OracleSourceConfig::RedStone(_) => {}
        // A Reflector quote source must itself be USD-based: this forbids a
        // quote chain and keeps the conversion exactly one hop. Read the base
        // cached when the quote market was configured (no live `base()` call).
        OracleSourceConfig::Reflector(r) => match &r.base {
            ReflectorBase::Usd => {}
            _ => panic_with_error!(env, OracleError::InvalidOracleBase),
        },
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
