//! Write path: storage, events, provider attestation, validate + probe + store.
//!
//! [`set_oracle`]: validate (static) → attest → probe (live) → commit.

use common::constants::{TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};
use common::errors::{GenericError, OracleError};
use common::oracle::observation::MIN_ORACLE_RESOLUTION_SECONDS;
use common::oracle::policy;
use common::oracle::providers::redstone::{xoxno_max_submission_age_call, REDSTONE_DECIMALS};
use common::oracle::providers::reflector::{
    reflector_base_call, reflector_decimals_call, reflector_resolution_call, ReflectorAsset,
};
use common::types::{
    AssetOracle, FeedSource, OracleTolerance, PriceKey, PriceSource, ProviderKind, ProviderRef,
};
use common::validation::{
    validate_oracle_tolerance, validate_sanity_bounds, validate_single_source_sanity_band,
};
use soroban_sdk::{
    assert_with_error, contractevent, contracttype, panic_with_error, Env, Symbol,
};

use crate::engine;
use crate::properties;
use crate::session::Session;

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

#[contracttype]
enum AggregatorKey {
    Oracle(PriceKey),
}

/// Stored oracle for `key`, renewing shared-tier TTL on hit.
pub(crate) fn get_oracle(env: &Env, key: &PriceKey) -> Option<AssetOracle> {
    let storage_key = AggregatorKey::Oracle(key.clone());
    let oracle: Option<AssetOracle> = env.storage().persistent().get(&storage_key);
    if oracle.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&storage_key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
    }
    oracle
}

pub(crate) fn store_oracle(env: &Env, key: &PriceKey, oracle: &AssetOracle) {
    let storage_key = AggregatorKey::Oracle(key.clone());
    env.storage().persistent().set(&storage_key, oracle);
    env.storage()
        .persistent()
        .extend_ttl(&storage_key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}

/// Test-only: remove a key's oracle.
#[cfg(any(test, feature = "testing"))]
pub(crate) fn remove_oracle(env: &Env, key: &PriceKey) {
    env.storage()
        .persistent()
        .remove(&AggregatorKey::Oracle(key.clone()));
}

fn commit(env: &Env, key: &PriceKey, oracle: &AssetOracle) {
    store_oracle(env, key, oracle);
    emit_updated(env, key, oracle);
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// Published on every composable-oracle write (config verbatim).
#[contractevent(topics = ["config", "asset_oracle"])]
#[derive(Clone, Debug)]
pub struct UpdateAssetOracleEvent {
    pub key: PriceKey,
    pub oracle: AssetOracle,
}

fn emit_updated(env: &Env, key: &PriceKey, oracle: &AssetOracle) {
    UpdateAssetOracleEvent {
        key: key.clone(),
        oracle: oracle.clone(),
    }
    .publish(env);
}

// ---------------------------------------------------------------------------
// Attest (configure-time provider facts)
// ---------------------------------------------------------------------------

fn attest_sources(env: &Env, oracle: &AssetOracle) {
    for source in oracle.sources.iter() {
        match &source {
            PriceSource::Feed(feed) => attest_feed(env, feed),
            PriceSource::Scaled(scaled) => attest_feed(env, &scaled.factor),
            // Refused at validate_source_shape; never stored successfully.
            PriceSource::LpShare(_) => {}
        }
    }
}

fn attest_feed(env: &Env, feed: &FeedSource) {
    match &feed.provider {
        ProviderRef::Reflector(reflector) => attest_reflector(
            env,
            &reflector.contract,
            feed.decimals,
            feed.max_stale_seconds,
        ),
        ProviderRef::MultiFeed(multi_feed) => match multi_feed.kind {
            ProviderKind::RedStone => assert_with_error!(
                env,
                feed.decimals == REDSTONE_DECIMALS,
                OracleError::InvalidOracleDecimals
            ),
            ProviderKind::Xoxno => attest_xoxno(
                env,
                &multi_feed.contract,
                feed.decimals,
                feed.max_stale_seconds,
            ),
            ProviderKind::Reflector => {
                panic_with_error!(env, GenericError::InvalidExchangeSrc)
            }
        },
    }
}

fn attest_reflector(env: &Env, contract: &soroban_sdk::Address, decimals: u32, max_stale: u64) {
    match reflector_base_call(env, contract) {
        ReflectorAsset::Other(symbol) if symbol == Symbol::new(env, "USD") => {}
        _ => panic_with_error!(env, OracleError::InvalidOracleBase),
    }
    assert_with_error!(
        env,
        reflector_decimals_call(env, contract) == decimals,
        OracleError::InvalidOracleDecimals
    );
    let resolution = reflector_resolution_call(env, contract);
    assert_with_error!(
        env,
        resolution >= MIN_ORACLE_RESOLUTION_SECONDS && u64::from(resolution) <= max_stale,
        OracleError::InvalidOracleResolution
    );
}

fn attest_xoxno(env: &Env, contract: &soroban_sdk::Address, decimals: u32, max_stale: u64) {
    assert_with_error!(
        env,
        reflector_decimals_call(env, contract) == decimals,
        OracleError::InvalidOracleDecimals
    );
    let adapter_window = xoxno_max_submission_age_call(env, contract);
    assert_with_error!(
        env,
        max_stale >= adapter_window,
        OracleError::InvalidStalenessConfig
    );
}

// ---------------------------------------------------------------------------
// Writes
// ---------------------------------------------------------------------------

/// Validate → attest → live probe → store → event.
pub(crate) fn set_oracle(env: &Env, key: PriceKey, oracle: AssetOracle) {
    validate_asset_oracle(env, &key, &oracle);
    attest_sources(env, &oracle);
    let mut session = Session::new(env);
    engine::probe(&mut session, &key, &oracle);
    commit(env, &key, &oracle);
}

/// Static validation from config + dependency graph (no feed reads).
pub(crate) fn validate_asset_oracle(env: &Env, key: &PriceKey, oracle: &AssetOracle) {
    validate_sanity_bounds(
        env,
        oracle.min_sanity_price_wad,
        oracle.max_sanity_price_wad,
    );
    validate_single_source_sanity_band(
        env,
        oracle.is_dual(),
        oracle.min_sanity_price_wad,
        oracle.max_sanity_price_wad,
    );
    policy::validate_asset_decimals(env, key, oracle.asset_decimals);

    for source in oracle.sources.iter() {
        policy::validate_source_shape(env, &source);
    }

    let mut session = Session::new(env);
    session.push_key(key);
    let derived = properties::properties_of_config(&mut session, &oracle.sources);
    session.pop_key();

    policy::validate_composition_depth(env, &derived.first);
    if let Some(second) = derived.second.as_ref() {
        policy::validate_composition_depth(env, second);
    }
    policy::validate_staleness_envelope(env, oracle.max_price_stale_seconds, &derived.combined());
    policy::validate_smoothing(env, &derived.first, derived.second.as_ref());

    validate_oracle_tolerance(env, &oracle.tolerance);
    if let Some(second) = derived.second.as_ref() {
        policy::validate_independence(env, &derived.first, second, &oracle.independence);
    }
}

/// Walk the sanity band; new band must overlap old and contain live hard price.
pub(crate) fn set_sanity_band(env: &Env, key: PriceKey, min_wad: i128, max_wad: i128) {
    let mut oracle = get_oracle(env, &key)
        .unwrap_or_else(|| panic_with_error!(env, OracleError::OracleNotConfigured));

    validate_sanity_bounds(env, min_wad, max_wad);
    validate_single_source_sanity_band(env, oracle.is_dual(), min_wad, max_wad);
    assert_with_error!(
        env,
        min_wad < oracle.max_sanity_price_wad && max_wad > oracle.min_sanity_price_wad,
        OracleError::InvalidSanityBounds
    );
    oracle.min_sanity_price_wad = min_wad;
    oracle.max_sanity_price_wad = max_wad;

    let mut session = Session::new(env);
    engine::probe(&mut session, &key, &oracle);
    commit(env, &key, &oracle);
}

/// Replace the dual-source agreement band.
///
/// Live hard probe after the band is applied so a widen cannot unfreeze a dual
/// market on a contested midpoint without the current feeds agreeing under the
/// new band (same containment discipline as [`set_sanity_band`]).
pub(crate) fn set_tolerance(env: &Env, key: PriceKey, tolerance: OracleTolerance) {
    let mut oracle = get_oracle(env, &key)
        .unwrap_or_else(|| panic_with_error!(env, OracleError::OracleNotConfigured));
    validate_oracle_tolerance(env, &tolerance);
    oracle.tolerance = tolerance;
    let mut session = Session::new(env);
    engine::probe(&mut session, &key, &oracle);
    commit(env, &key, &oracle);
}

#[cfg(test)]
#[path = "../tests/oracle/registry.rs"]
mod tests;
