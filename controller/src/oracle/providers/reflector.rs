use common::errors::{GenericError, OracleError};
use common::types::{OracleAssetRef, OracleProviderKind, OracleReadMode, ReflectorSourceConfig};
use soroban_sdk::{panic_with_error, Env};

use super::super::observation::{
    check_not_future_at, is_stale, normalize_positive_price, OracleObservation,
};
use super::super::reflector::{
    reflector_lastprice_call, reflector_prices_call, ReflectorAsset, ReflectorPriceData,
};
use crate::cache::ControllerCache;

pub(crate) fn to_reflector_asset(env: &Env, asset: &OracleAssetRef) -> ReflectorAsset {
    match asset {
        OracleAssetRef::Stellar(address) => ReflectorAsset::Stellar(address.clone()),
        OracleAssetRef::Symbol(symbol) => ReflectorAsset::Other(symbol.clone()),
        OracleAssetRef::String(_) => panic_with_error!(env, OracleError::InvalidOracleTokenType),
    }
}

pub(crate) fn min_twap_observations(records: u32) -> u32 {
    core::cmp::max(1, records.div_ceil(2))
}

pub(crate) fn read_reflector_source(
    cache: &mut ControllerCache,
    config: &ReflectorSourceConfig,
    max_stale: u64,
    required: bool,
) -> Option<OracleObservation> {
    match config.read_mode {
        OracleReadMode::Spot => read_spot(cache, config, required),
        OracleReadMode::Twap(records) => read_twap(cache, config, records, max_stale, required),
    }
}

fn read_spot(
    cache: &mut ControllerCache,
    config: &ReflectorSourceConfig,
    required: bool,
) -> Option<OracleObservation> {
    let env = cache.env();
    let asset = to_reflector_asset(env, &config.asset);
    let Some(pd) = reflector_lastprice_call(env, &config.contract, &asset) else {
        if required {
            panic_with_error!(env, OracleError::NoLastPrice);
        }
        return None;
    };
    Some(observation_from_price_data(
        env,
        &pd,
        config.decimals,
        config.read_mode.clone(),
    ))
}

fn read_twap(
    cache: &mut ControllerCache,
    config: &ReflectorSourceConfig,
    records: u32,
    max_stale: u64,
    required: bool,
) -> Option<OracleObservation> {
    let spot = read_spot(cache, config, required)?;
    if records == 0 {
        return twap_fallback_or_panic(cache, spot, OracleError::TwapInsufficientObservations);
    }

    let env = cache.env();
    let asset = to_reflector_asset(env, &config.asset);
    let Some(history) = reflector_prices_call(env, &config.contract, &asset, records) else {
        return twap_fallback_or_panic(cache, spot, OracleError::ReflectorHistoryEmpty);
    };
    if history.is_empty() {
        return twap_fallback_or_panic(cache, spot, OracleError::ReflectorHistoryEmpty);
    }
    if history.len() < min_twap_observations(records) {
        return twap_fallback_or_panic(cache, spot, OracleError::TwapInsufficientObservations);
    }

    let mut sum: i128 = 0;
    let mut oldest_ts = u64::MAX;
    for pd in history.iter() {
        check_not_future_at(env, cache.current_timestamp_ms / 1000, pd.timestamp);
        if pd.price <= 0 {
            return twap_fallback_or_panic(cache, spot, OracleError::InvalidPrice);
        }
        sum = sum
            .checked_add(pd.price)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
        if pd.timestamp < oldest_ts {
            oldest_ts = pd.timestamp;
        }
    }

    if is_stale(cache.current_timestamp_ms / 1000, oldest_ts, max_stale) {
        return twap_fallback_or_panic(cache, spot, OracleError::PriceFeedStale);
    }

    let raw_price = sum / history.len() as i128;
    Some(OracleObservation {
        price_wad: normalize_positive_price(env, raw_price, config.decimals),
        raw_price,
        raw_decimals: config.decimals,
        observed_at: oldest_ts,
        published_at: None,
        provider: OracleProviderKind::ReflectorSep40,
        read_mode: OracleReadMode::Twap(records),
    })
}

fn twap_fallback_or_panic(
    cache: &ControllerCache,
    spot: OracleObservation,
    err: OracleError,
) -> Option<OracleObservation> {
    if cache.oracle_policy.allows_missing_twap_fallback() {
        Some(spot)
    } else {
        panic_with_error!(cache.env(), err);
    }
}

fn observation_from_price_data(
    env: &Env,
    pd: &ReflectorPriceData,
    decimals: u32,
    read_mode: OracleReadMode,
) -> OracleObservation {
    check_not_future_at(env, env.ledger().timestamp(), pd.timestamp);
    OracleObservation {
        price_wad: normalize_positive_price(env, pd.price, decimals),
        raw_price: pd.price,
        raw_decimals: decimals,
        observed_at: pd.timestamp,
        published_at: None,
        provider: OracleProviderKind::ReflectorSep40,
        read_mode,
    }
}
