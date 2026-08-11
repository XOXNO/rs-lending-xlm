//! Validation checks applied to oracle configuration when a price key is
//! registered: source counts, composition depth, staleness bounds,
//! smoothing/independence policy, source shape, and asset decimals. Each
//! check panics with the corresponding `OracleError` or `GenericError`
//! variant when the configuration is invalid.

use common::constants::{MAX_ASSET_DECIMALS, MIN_ASSET_DECIMALS};
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

const MIN_SMOOTHING_TWAP_RECORDS: u32 = 2;

/// Panics with `OracleError::SourceCountOutOfRange` unless `count` is
/// within `MIN_SOURCES..=MAX_SOURCES`.
pub(crate) fn source_count(env: &Env, count: u32) {
    if !(MIN_SOURCES..=MAX_SOURCES).contains(&count) {
        panic_with_error!(env, OracleError::SourceCountOutOfRange);
    }
}

/// Panics with `OracleError::OracleDepthExceeded` if `properties.depth`
/// exceeds `MAX_RESOLUTION_DEPTH`.
pub(crate) fn composition_depth(env: &Env, properties: &SourceProperties) {
    if properties.depth > MAX_RESOLUTION_DEPTH {
        panic_with_error!(env, OracleError::OracleDepthExceeded);
    }
}

/// Panics with `OracleError::InvalidStalenessConfig` if
/// `asset_max_stale_seconds` falls outside
/// `MIN_PRICE_STALE_SECONDS..=MAX_PRICE_STALE_SECONDS`, or if any source's
/// staleness bound (`properties.loosest_max_stale_seconds`) is looser than
/// the asset's configured bound.
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

/// Panics with `GenericError::SpotOnlyNotProductionSafe` if `first` has an
/// unsmoothed market leg and `second` is either absent or also has an
/// unsmoothed market leg.
pub(crate) fn smoothing(env: &Env, first: &SourceProperties, second: Option<&SourceProperties>) {
    if first.has_unsmoothed_market_leg
        && second.is_none_or(|source| source.has_unsmoothed_market_leg)
    {
        panic_with_error!(env, GenericError::SpotOnlyNotProductionSafe);
    }
}

/// Checks the contracts shared between `first` and `second` against
/// `policy`, panicking with `OracleError::IndependenceNotDeclared` if
/// `policy` requires disjoint sources and any contract is shared, or if
/// `policy` allows a declared shared set that does not exactly match the
/// contracts actually shared between the two.
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

/// Returns whether `left` and `right` contain exactly the same addresses,
/// irrespective of order or duplicates.
fn same_address_set(left: &Vec<Address>, right: &Vec<Address>) -> bool {
    left.iter().all(|address| right.contains(&address))
        && right.iter().all(|address| left.contains(&address))
}

/// Validates the shape of a single price source, dispatching to the
/// feed-, factor-, or LP-specific checks depending on the source variant.
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

/// Validates an Aquarius LP source. Panics with
/// `OracleError::InvalidOracleDecimals` if either reserve's decimals fall
/// outside `MIN_ASSET_DECIMALS..=MAX_ASSET_DECIMALS`. Panics with
/// `OracleError::InvalidOracleBase` if the two pool tokens or their price
/// keys are identical, if either key does not price its paired token, or
/// if `min_pool_value_wad` is not positive.
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

/// Returns whether `key` is consistent with pricing `token`: a `Token` key
/// must reference `token` itself, while a `Ref` key is treated as valid for
/// any token.
fn key_prices_token(key: &PriceKey, token: &Address) -> bool {
    match key {
        PriceKey::Token(address) => address == token,
        PriceKey::Ref(_) => true,
    }
}

/// Validates a single feed source. Panics with
/// `OracleError::InvalidOracleDecimals` if `feed.decimals` falls outside
/// `MIN_ORACLE_DECIMALS..=MAX_ORACLE_DECIMALS`, and with
/// `OracleError::InvalidStalenessConfig` if `feed.max_stale_seconds` falls
/// outside `MIN_PRICE_STALE_SECONDS..=MAX_PRICE_STALE_SECONDS`. For a
/// Reflector provider in TWAP read mode, also validates the TWAP record
/// count via `validate_twap_records` and panics with
/// `OracleError::TwapInsufficientObservations` if it is below
/// `MIN_SMOOTHING_TWAP_RECORDS`.
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

/// Validates the decimals declared for a price key. A `Token` key must
/// declare decimals within `MIN_ASSET_DECIMALS..=MAX_ASSET_DECIMALS`; a
/// `Ref` key must declare exactly `0`. Panics with
/// `OracleError::InvalidOracleDecimals` otherwise.
pub(crate) fn asset_decimals(env: &Env, key: &PriceKey, decimals: u32) {
    let valid = match key {
        PriceKey::Token(_) => (MIN_ASSET_DECIMALS..=MAX_ASSET_DECIMALS).contains(&decimals),
        PriceKey::Ref(_) => decimals == 0,
    };
    if !valid {
        panic_with_error!(env, OracleError::InvalidOracleDecimals);
    }
}

/// Validates the min/max factor bounds of a scaled source. Panics with
/// `OracleError::InvalidSanityBounds` if `min_factor_wad` is not positive,
/// if `max_factor_wad` is below `min_factor_wad`, or if `max_factor_wad`
/// exceeds `MAX_REASONABLE_PRICE_WAD`.
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
