//! Oracle configuration writes and their validation. Callers are owner-gated in
//! `lib.rs`; the resolved [`AssetOracle`] arrives pre-built, with
//! `asset_decimals` sourced from the token by the governance resolver.

use common::errors::OracleError;
use common::oracle::policy;
use common::types::{AssetOracle, OracleStrategy, OracleTolerance, PriceKey};
use common::validation::{
    validate_oracle_tolerance, validate_sanity_bounds, validate_single_source_sanity_band,
};
use soroban_sdk::{assert_with_error, panic_with_error, Env};

use crate::context::ResolutionContext;
use crate::engine;
use crate::events::emit_asset_oracle_updated;
use crate::properties;
use crate::registry;

// ---------------------------------------------------------------------------
// Composable model
// ---------------------------------------------------------------------------

/// Validates and stores an [`AssetOracle`] under `key`.
///
/// Every rule is a predicate over derived [`SourceProperties`]; none inspects a
/// provider variant. That is what lets a new provider or composition shape land
/// without editing this function.
///
/// Validation walks the dependency graph, so it is only as current as the graph
/// is *now*. A dependency re-pointed later can invalidate the independence and
/// depth conclusions drawn here, which is why the read path re-checks depth and
/// the cycle guard rather than trusting this call.
///
/// # Errors
/// * [`OracleError::SourceCountOutOfRange`] - not one or two sources.
/// * [`OracleError::OracleDepthExceeded`] - composition nested past the cap.
/// * [`OracleError::InvalidStalenessConfig`] - ceiling out of range, or a
///   component permitted to outlive it.
/// * [`GenericError::SpotOnlyNotProductionSafe`] - every opinion is movable by
///   trading without a window.
/// * [`OracleError::IndependenceNotDeclared`] - shared trust does not match the
///   declared policy.
/// * [`OracleError::InvalidSanityBounds`] - malformed sanity or factor bounds.
pub(crate) fn set_asset_oracle(env: &Env, key: PriceKey, oracle: AssetOracle) {
    validate_sanity_bounds(
        env,
        oracle.min_sanity_price_wad,
        oracle.max_sanity_price_wad,
    );
    validate_single_source_sanity_band_for(env, &oracle);
    policy::validate_asset_decimals(env, &key, oracle.asset_decimals);

    // Provider-level shape: decimals, the TWAP window, factor bounds, and the
    // refusal of source kinds the engine cannot price. v1 read decimals off the
    // provider contract and bounded the window at listing time; the composable
    // model takes both as config data, so they are bounded here or nowhere.
    for source in oracle.sources.iter() {
        policy::validate_source_shape(env, &source);
    }

    let mut cache = ResolutionContext::new(env);
    // Pushed so a config naming itself as its own dependency is rejected now,
    // with a cycle error, rather than storing and bricking on first read.
    cache.push_price_key(&key);
    let derived = properties::properties_of_config(&mut cache, &oracle.sources);
    cache.pop_price_key();

    policy::validate_composition_depth(env, &derived.first);
    if let Some(second) = derived.second.as_ref() {
        policy::validate_composition_depth(env, second);
    }
    policy::validate_staleness_envelope(env, oracle.max_price_stale_seconds, &derived.combined());
    policy::validate_smoothing(env, &derived.first, derived.second.as_ref());

    if let Some(second) = derived.second.as_ref() {
        validate_oracle_tolerance(env, &oracle.tolerance);
        policy::validate_independence(env, &derived.first, second, &oracle.independence);
    }

    registry::set_oracle(env, &key, &oracle);
    emit_asset_oracle_updated(env, &key, &oracle);
}

/// A lone opinion has nothing to be checked against, so its sanity band is the
/// only backstop and must stay narrow. Same rule as v1, keyed off the source
/// count instead of a strategy enum that no longer exists.
fn validate_single_source_sanity_band_for(env: &Env, oracle: &AssetOracle) {
    let strategy = if oracle.is_dual() {
        OracleStrategy::PrimaryWithAnchor
    } else {
        OracleStrategy::Single
    };
    validate_single_source_sanity_band(
        env,
        strategy,
        oracle.min_sanity_price_wad,
        oracle.max_sanity_price_wad,
    );
}

/// Moves only the sanity band on an active oracle, keeping every other field.
///
/// The new band must overlap the old one and contain the current live price,
/// proven by resolving it under the new band before anything is stored: a band
/// can be walked, never teleported to a disjoint range on one transient print.
pub(crate) fn set_sanity_band(env: &Env, key: PriceKey, min_wad: i128, max_wad: i128) {
    let mut oracle = registry::resolve_oracle(env, &key)
        .unwrap_or_else(|| panic_with_error!(env, OracleError::OracleNotConfigured));

    validate_sanity_bounds(env, min_wad, max_wad);
    validate_single_source_sanity_band_for_band(env, &oracle, min_wad, max_wad);
    assert_with_error!(
        env,
        min_wad < oracle.max_sanity_price_wad && max_wad > oracle.min_sanity_price_wad,
        OracleError::InvalidSanityBounds
    );
    oracle.min_sanity_price_wad = min_wad;
    oracle.max_sanity_price_wad = max_wad;

    // Containment probe under the updated config; nothing is stored unless the
    // live price sits inside the new band.
    let mut cache = ResolutionContext::new(env);
    let _ = engine::resolve_probe(&mut cache, &key, &oracle);

    registry::set_oracle(env, &key, &oracle);
    emit_asset_oracle_updated(env, &key, &oracle);
}

/// Replaces the agreement band on an active oracle after envelope validation.
///
/// Re-validated here rather than trusted from the caller: a direct owner call
/// must not be able to store a degenerate band that disables the guard.
pub(crate) fn set_tolerance(env: &Env, key: PriceKey, tolerance: OracleTolerance) {
    let mut oracle = registry::resolve_oracle(env, &key)
        .unwrap_or_else(|| panic_with_error!(env, OracleError::OracleNotConfigured));
    validate_oracle_tolerance(env, &tolerance);
    oracle.tolerance = tolerance;
    registry::set_oracle(env, &key, &oracle);
    emit_asset_oracle_updated(env, &key, &oracle);
}

fn validate_single_source_sanity_band_for_band(
    env: &Env,
    oracle: &AssetOracle,
    min_wad: i128,
    max_wad: i128,
) {
    let strategy = if oracle.is_dual() {
        OracleStrategy::PrimaryWithAnchor
    } else {
        OracleStrategy::Single
    };
    validate_single_source_sanity_band(env, strategy, min_wad, max_wad);
}
