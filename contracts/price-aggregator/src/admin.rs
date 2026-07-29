//! Write path: persistent storage, events, provider attestation, and
//! validate → probe → commit.
//!
//! [`set_oracle`]: static validate → attest live provider facts → hard probe →
//! store + event. [`set_sanity_band`] and [`set_tolerance`] stage the field
//! change, hard-probe under the staged config, then commit.

use common::constants::{TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};
use common::errors::{GenericError, OracleError};
use common::oracle::observation::MIN_ORACLE_RESOLUTION_SECONDS;
use common::oracle::policy;
use common::oracle::providers::redstone::{xoxno_max_submission_age_call, REDSTONE_DECIMALS};
use common::oracle::providers::reflector::{
    reflector_base_call, reflector_decimals_call, reflector_resolution_call, ReflectorAsset,
};
use common::types::{
    local_properties, AssetOracle, FeedSource, OracleReadMode, OracleTolerance, PriceKey,
    PriceSource, ProviderKind, ProviderRef, ReflectorFeedRef,
};
use common::validation::{
    validate_oracle_tolerance, validate_sanity_bounds, validate_single_source_sanity_band,
};
use soroban_sdk::{
    assert_with_error, contractevent, contracttype, panic_with_error, Env, Symbol, Vec,
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
    OracleKeys,
}

fn oracle_keys(env: &Env) -> Vec<PriceKey> {
    env.storage()
        .instance()
        .get(&AggregatorKey::OracleKeys)
        .unwrap_or_else(|| Vec::new(env))
}

fn store_oracle_keys(env: &Env, keys: &Vec<PriceKey>) {
    env.storage()
        .instance()
        .set(&AggregatorKey::OracleKeys, keys);
}

/// Stored oracle for `key`, extending shared-tier TTL on hit.
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

/// Persist `oracle` for `key` and extend shared-tier TTL.
pub(crate) fn store_oracle(env: &Env, key: &PriceKey, oracle: &AssetOracle) {
    let storage_key = AggregatorKey::Oracle(key.clone());
    env.storage().persistent().set(&storage_key, oracle);
    env.storage()
        .persistent()
        .extend_ttl(&storage_key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
    let mut keys = oracle_keys(env);
    if keys.first_index_of(key).is_none() {
        keys.push_back(key.clone());
        store_oracle_keys(env, &keys);
    }
}

/// Test-only: remove the stored oracle for `key`.
#[cfg(any(test, feature = "testing"))]
pub(crate) fn remove_oracle(env: &Env, key: &PriceKey) {
    env.storage()
        .persistent()
        .remove(&AggregatorKey::Oracle(key.clone()));
    let mut keys = oracle_keys(env);
    if let Some(index) = keys.first_index_of(key) {
        keys.remove(index);
        store_oracle_keys(env, &keys);
    }
}

fn commit(env: &Env, key: &PriceKey, oracle: &AssetOracle) {
    store_oracle(env, key, oracle);
    emit_updated(env, key, oracle);
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// Emitted on every successful oracle write with the stored config verbatim.
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
        ProviderRef::Reflector(reflector) => {
            attest_reflector(env, reflector, feed.decimals, feed.max_stale_seconds)
        }
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

fn attest_reflector(env: &Env, feed: &ReflectorFeedRef, decimals: u32, max_stale: u64) {
    match reflector_base_call(env, &feed.contract) {
        ReflectorAsset::Other(symbol) if symbol == Symbol::new(env, "USD") => {}
        _ => panic_with_error!(env, OracleError::InvalidOracleBase),
    }
    assert_with_error!(
        env,
        reflector_decimals_call(env, &feed.contract) == decimals,
        OracleError::InvalidOracleDecimals
    );
    let resolution = reflector_resolution_call(env, &feed.contract);
    assert_with_error!(
        env,
        resolution >= MIN_ORACLE_RESOLUTION_SECONDS && u64::from(resolution) <= max_stale,
        OracleError::InvalidOracleResolution
    );
    if let OracleReadMode::Twap(records) = feed.read_mode {
        let required_span =
            u64::from(records.saturating_sub(1)).saturating_mul(u64::from(resolution));
        assert_with_error!(
            env,
            required_span <= max_stale,
            OracleError::InvalidOracleResolution
        );
    }
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

/// Validate → attest → live hard probe → store → event.
///
/// # Errors
/// * Static config validation ([`validate_asset_oracle`]).
/// * Provider attestation (base, decimals, resolution, staleness envelope).
/// * Hard probe gate failures under the staged config.
///
/// # Events
/// * [`UpdateAssetOracleEvent`]
pub(crate) fn set_oracle(env: &Env, key: PriceKey, oracle: AssetOracle) {
    validate_asset_oracle(env, &key, &oracle);
    attest_sources(env, &oracle);
    let mut session = Session::new(env);
    engine::probe(&mut session, &key, &oracle);
    // Stage before validating ancestors. Any panic rolls the entire contract
    // call back, so a rejected child update leaves storage unchanged.
    store_oracle(env, &key, &oracle);
    revalidate_dependents(env, &key);
    emit_updated(env, &key, &oracle);
}

fn revalidate_dependents(env: &Env, changed: &PriceKey) {
    for candidate in oracle_keys(env).iter() {
        if candidate == *changed || !depends_on(env, &candidate, changed, &mut Vec::new(env)) {
            continue;
        }
        let oracle = get_oracle(env, &candidate)
            .unwrap_or_else(|| panic_with_error!(env, OracleError::OracleNotConfigured));
        validate_asset_oracle(env, &candidate, &oracle);
        let mut session = Session::new(env);
        engine::probe(&mut session, &candidate, &oracle);
    }
}

fn depends_on(env: &Env, root: &PriceKey, target: &PriceKey, visiting: &mut Vec<PriceKey>) -> bool {
    if visiting.first_index_of(root).is_some() {
        return false;
    }
    visiting.push_back(root.clone());
    let found = get_oracle(env, root).is_some_and(|oracle| {
        oracle.sources.iter().any(|source| {
            local_properties(env, &source)
                .dependencies
                .iter()
                .any(|dependency| {
                    dependency == *target || depends_on(env, &dependency, target, visiting)
                })
        })
    });
    visiting.pop_back();
    found
}

/// Static config validation and dependency-graph checks (no feed reads).
///
/// Walks sources for shape, composition depth, staleness envelope, smoothing,
/// tolerance, and dual-source independence. Builds properties via
/// [`properties::properties_of_config`], which panics if a nested key lacks a
/// stored oracle or the dependency graph cycles / exceeds depth.
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

/// Stage a new sanity band, require overlap with the previous band, hard-probe
/// so the live price lies inside the new band, then commit.
///
/// # Errors
/// * [`OracleError::OracleNotConfigured`] — no stored oracle for `key`.
/// * [`OracleError::InvalidSanityBounds`] — invalid or non-overlapping band;
///   single-source band rules.
/// * Hard probe failures under the staged band.
///
/// # Events
/// * [`UpdateAssetOracleEvent`]
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

/// Replace the dual-source agreement band, hard-probe under the staged band,
/// then commit.
///
/// Probe runs after the band is applied so a widen cannot commit a contested
/// midpoint that still fails under the new band.
///
/// # Errors
/// * [`OracleError::OracleNotConfigured`] — no stored oracle for `key`.
/// * Tolerance validation and hard probe failures.
///
/// # Events
/// * [`UpdateAssetOracleEvent`]
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
