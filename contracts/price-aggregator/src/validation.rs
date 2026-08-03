use common::errors::{GenericError, OracleError};
use common::oracle::observation::{
    MAX_ORACLE_DECIMALS, MAX_PRICE_STALE_SECONDS, MIN_ORACLE_DECIMALS, MIN_PRICE_STALE_SECONDS,
};
use common::types::{
    AquariusLpSource, FeedSource, IndependencePolicy, OracleReadMode, PriceKey, PriceSource,
    ProviderRef, ScaledSource, MAX_RESOLUTION_DEPTH, MAX_SOURCES, MIN_SOURCES,
};
use common::validation::validate_twap_records;
use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::properties::SourceProperties;

const MIN_ASSET_DECIMALS: u32 = 3;
const MAX_ASSET_DECIMALS: u32 = 18;
const MIN_SMOOTHING_TWAP_RECORDS: u32 = 2;

pub(crate) fn source_count(env: &Env, count: u32) {
    if !(MIN_SOURCES..=MAX_SOURCES).contains(&count) {
        panic_with_error!(env, OracleError::SourceCountOutOfRange);
    }
}

pub(crate) fn composition_depth(env: &Env, properties: &SourceProperties) {
    if properties.depth > MAX_RESOLUTION_DEPTH {
        panic_with_error!(env, OracleError::OracleDepthExceeded);
    }
}

pub(crate) fn staleness_envelope(
    env: &Env,
    asset_max_stale_seconds: u64,
    properties: &SourceProperties,
) {
    if !(MIN_PRICE_STALE_SECONDS..=MAX_PRICE_STALE_SECONDS).contains(&asset_max_stale_seconds)
        || properties.loosest_max_stale_seconds > asset_max_stale_seconds
    {
        panic_with_error!(env, OracleError::InvalidStalenessConfig);
    }
}

pub(crate) fn smoothing(env: &Env, first: &SourceProperties, second: Option<&SourceProperties>) {
    if first.has_unsmoothed_market_leg
        && second.is_none_or(|source| source.has_unsmoothed_market_leg)
    {
        panic_with_error!(env, GenericError::SpotOnlyNotProductionSafe);
    }
}

pub(crate) fn independence(
    env: &Env,
    first: &SourceProperties,
    second: &SourceProperties,
    policy: &IndependencePolicy,
) {
    let shared = first.shared_contracts_with(env, second);
    match policy {
        IndependencePolicy::RequireDisjoint if !shared.is_empty() => {
            panic_with_error!(env, OracleError::IndependenceNotDeclared)
        }
        IndependencePolicy::AllowShared(declared)
            if declared.is_empty() || !same_address_set(&shared, declared) =>
        {
            panic_with_error!(env, OracleError::IndependenceNotDeclared)
        }
        _ => {}
    }
}

fn same_address_set(left: &Vec<Address>, right: &Vec<Address>) -> bool {
    left.iter().all(|address| right.contains(&address))
        && right.iter().all(|address| left.contains(&address))
}

pub(crate) fn source_shape(env: &Env, source: &PriceSource) {
    match source {
        PriceSource::Feed(feed) => feed_shape(env, feed),
        PriceSource::Scaled(scaled) => {
            feed_shape(env, &scaled.factor);
            factor_bounds(env, scaled);
        }
        PriceSource::AquariusLp(lp) | PriceSource::AquariusStableLp(lp) => {
            aquarius_lp_shape(env, lp)
        }
    }
}

fn aquarius_lp_shape(env: &Env, lp: &AquariusLpSource) {
    if ![lp.reserve_a_decimals, lp.reserve_b_decimals]
        .iter()
        .all(|decimals| (MIN_ASSET_DECIMALS..=MAX_ASSET_DECIMALS).contains(decimals))
    {
        panic_with_error!(env, OracleError::InvalidOracleDecimals);
    }
    if lp.token_a == lp.token_b
        || lp.key_a == lp.key_b
        || !key_prices_token(&lp.key_a, &lp.token_a)
        || !key_prices_token(&lp.key_b, &lp.token_b)
        || lp.min_pool_value_wad <= 0
    {
        panic_with_error!(env, OracleError::InvalidOracleBase);
    }
}

fn key_prices_token(key: &PriceKey, token: &Address) -> bool {
    match key {
        PriceKey::Token(address) => address == token,
        PriceKey::Ref(_) => true,
    }
}

fn feed_shape(env: &Env, feed: &FeedSource) {
    if !(MIN_ORACLE_DECIMALS..=MAX_ORACLE_DECIMALS).contains(&feed.decimals) {
        panic_with_error!(env, OracleError::InvalidOracleDecimals);
    }
    if !(MIN_PRICE_STALE_SECONDS..=MAX_PRICE_STALE_SECONDS).contains(&feed.max_stale_seconds) {
        panic_with_error!(env, OracleError::InvalidStalenessConfig);
    }
    if let ProviderRef::Reflector(reflector) = &feed.provider {
        if let OracleReadMode::Twap(records) = reflector.read_mode {
            validate_twap_records(env, records);
            if records < MIN_SMOOTHING_TWAP_RECORDS {
                panic_with_error!(env, OracleError::TwapInsufficientObservations);
            }
        }
    }
}

pub(crate) fn asset_decimals(env: &Env, key: &PriceKey, decimals: u32) {
    let valid = match key {
        PriceKey::Token(_) => (MIN_ASSET_DECIMALS..=MAX_ASSET_DECIMALS).contains(&decimals),
        PriceKey::Ref(_) => decimals == 0,
    };
    if !valid {
        panic_with_error!(env, OracleError::InvalidOracleDecimals);
    }
}

fn factor_bounds(env: &Env, scaled: &ScaledSource) {
    if scaled.min_factor_wad <= 0
        || scaled.max_factor_wad < scaled.min_factor_wad
        || scaled.max_factor_wad > common::constants::MAX_REASONABLE_PRICE_WAD
    {
        panic_with_error!(env, OracleError::InvalidSanityBounds);
    }
}

#[cfg(test)]
#[path = "../tests/oracle/validation.rs"]
mod tests;
