//! Administrative operations for configuring, revalidating, and updating asset oracles.
//!
//! Provides the entry points used by privileged callers to register a new oracle
//! configuration, attest its price sources, adjust its sanity bounds, or adjust its
//! tolerance, and cascades revalidation to any other configured oracle whose
//! composition depends on the changed key.

use common::errors::OracleError;
use common::types::{AssetOracle, FeedSource, OracleTolerance, PriceKey, PriceSource, ProviderRef};
use common::validation::{
    validate_oracle_tolerance, validate_sanity_bounds, validate_single_source_sanity_band,
};
use soroban_sdk::{assert_with_error, panic_with_error, Env, Vec};

use crate::engine;
use crate::properties;
use crate::providers::{aquarius, redstone, reflector, xoxno};
use crate::registry;
use crate::session::Session;
use crate::validation;

/// Attests every price source configured on `oracle`, dispatching feed, scaled-feed,
/// Aquarius LP, and Aquarius stable-LP sources to their respective provider
/// attestation routines.
fn attest_sources(env: &Env, key: &PriceKey, oracle: &AssetOracle) {
    for source in oracle.sources.iter() {
        match &source {
            PriceSource::Feed(feed) => attest_feed(env, feed),
            PriceSource::Scaled(scaled) => attest_feed(env, &scaled.factor),

            PriceSource::AquariusLp(lp) => aquarius::attest(env, key, oracle, lp, false),
            PriceSource::AquariusStableLp(lp) => aquarius::attest(env, key, oracle, lp, true),
        }
    }
}

/// Attests a single feed source by dispatching to the provider-specific attestation
/// routine identified by `feed.provider`.
fn attest_feed(env: &Env, feed: &FeedSource) {
    match &feed.provider {
        ProviderRef::Reflector(reflector) => {
            reflector::attest(env, reflector, feed.decimals, feed.max_stale_seconds)
        }
        ProviderRef::RedStone(_) => redstone::attest(env, feed.decimals),
        ProviderRef::Xoxno(xoxno_feed) => {
            xoxno::attest(env, xoxno_feed, feed.decimals, feed.max_stale_seconds)
        }
    }
}

/// Validates `oracle` and attests its sources, then probes it: a hard probe
/// for Aquarius LP oracles that panics on any unusable outcome, or a soft
/// probe otherwise that panics only on configuration-level failures. Stores
/// the oracle, revalidates dependents, and emits the registry event.
pub(crate) fn set_oracle(env: &Env, key: PriceKey, oracle: AssetOracle) {
    validate_asset_oracle(env, &key, &oracle);
    attest_sources(env, &key, &oracle);
    let mut session = Session::new(env);
    if oracle.has_aquarius_lp_source() {
        engine::probe_priceable(&mut session, &key, &oracle);
    } else {
        engine::probe(&mut session, &key, &oracle);
    }

    registry::store_oracle(env, &key, &oracle);
    revalidate_dependents(env, &key);
    registry::emit(env, &key, &oracle);
}

/// Revalidates and re-probes every oracle registered in the registry whose
/// composition transitively depends on `changed`, panicking if any of them fails
/// validation under the new state.
fn revalidate_dependents(env: &Env, changed: &PriceKey) {
    for candidate in registry::oracle_keys(env).iter() {
        if candidate == *changed || !depends_on(env, &candidate, changed, &mut Vec::new(env)) {
            continue;
        }
        let oracle = registry::get_oracle(env, &candidate)
            .unwrap_or_else(|| panic_with_error!(env, OracleError::OracleNotConfigured));
        validate_asset_oracle(env, &candidate, &oracle);
        let mut session = Session::new(env);
        engine::probe(&mut session, &candidate, &oracle);
    }
}

/// Returns whether `root`'s registered oracle composition depends, directly or
/// transitively, on `target`. Guards against cycles in the dependency graph via
/// `visiting`, treating a key already on the current path as a non-match.
fn depends_on(env: &Env, root: &PriceKey, target: &PriceKey, visiting: &mut Vec<PriceKey>) -> bool {
    if visiting.first_index_of(root).is_some() {
        return false;
    }
    visiting.push_back(root.clone());
    let found = registry::get_oracle(env, root).is_some_and(|oracle| {
        oracle.sources.iter().any(|source| {
            properties::local_properties(env, &source)
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

/// Runs the full validation suite for `oracle` under `key`, covering sanity
/// bounds, source shape and count, asset decimals, composition depth,
/// staleness, smoothing, tolerance, and source independence, with the
/// smoothing and tolerance checks waived for Aquarius LP oracles. Panics if
/// any check fails.
pub(crate) fn validate_asset_oracle(env: &Env, key: &PriceKey, oracle: &AssetOracle) {
    let mut session = Session::new(env);
    session.push_key(key);
    let derived = properties::properties_of_config(&mut session, &oracle.sources);
    session.pop_key();

    validate_sanity_bounds(
        env,
        oracle.min_sanity_price_wad,
        oracle.max_sanity_price_wad,
    );
    if oracle.has_aquarius_lp_source() {
        // Sole-source by construction, so the band is the only backstop.
        common::validation::validate_lp_sanity_band(
            env,
            oracle.min_sanity_price_wad,
            oracle.max_sanity_price_wad,
        );
    } else {
        let exempt_from_band_cap = derived.second.as_ref().is_some_and(|second| {
            !validation::same_address_set(&derived.first.trust, &second.trust)
        });
        validate_single_source_sanity_band(
            env,
            exempt_from_band_cap,
            oracle.min_sanity_price_wad,
            oracle.max_sanity_price_wad,
        );
    }
    validation::asset_decimals(env, key, oracle.asset_decimals);

    for source in oracle.sources.iter() {
        validation::source_shape(env, &source);
    }

    let has_lp = oracle.sources.iter().any(|source| source.is_aquarius_lp());
    if has_lp && oracle.sources.len() != 1 {
        panic_with_error!(env, OracleError::SourceCountOutOfRange);
    }

    validation::composition_depth(env, &derived.first);
    if let Some(second) = derived.second.as_ref() {
        validation::composition_depth(env, second);
    }
    validation::staleness_envelope(env, oracle.max_price_stale_seconds, &derived.combined());
    if !oracle.has_aquarius_lp_source() {
        validation::smoothing(env, &derived.first, derived.second.as_ref());
    }

    if !oracle.has_aquarius_lp_source() {
        validate_oracle_tolerance(env, &oracle.tolerance);
    }
    if let Some(second) = derived.second.as_ref() {
        validation::independence(env, &derived.first, second, &oracle.independence);
    }
}

/// Tightens the sanity band on the immediate (no-timelock) path: it may only
/// narrow, never widen. Widening must go through the timelocked
/// `ConfigureAssetOracle` so it has a reaction window; without this ratchet the
/// old intersect-only check let `ORACLE_ROLE` walk the band across calls (F-3,
/// INV-AUTH-04). Revalidates and re-probes before committing.
pub(crate) fn set_sanity_band(env: &Env, key: PriceKey, min_wad: i128, max_wad: i128) {
    let mut oracle = registry::get_oracle(env, &key)
        .unwrap_or_else(|| panic_with_error!(env, OracleError::OracleNotConfigured));

    assert_with_error!(
        env,
        min_wad >= oracle.min_sanity_price_wad && max_wad <= oracle.max_sanity_price_wad,
        OracleError::SanityBandMustTighten
    );
    oracle.min_sanity_price_wad = min_wad;
    oracle.max_sanity_price_wad = max_wad;
    validate_asset_oracle(env, &key, &oracle);

    let mut session = Session::new(env);
    engine::probe(&mut session, &key, &oracle);
    registry::commit(env, &key, &oracle);
}

/// Updates the tolerance of the oracle registered under `key`. Rejects Aquarius LP
/// oracles, validates the new tolerance, re-probes the oracle, and commits the
/// result to the registry. Panics if the oracle is not configured, has an
/// Aquarius LP source, or the tolerance fails validation.
pub(crate) fn set_tolerance(env: &Env, key: PriceKey, tolerance: OracleTolerance) {
    let mut oracle = registry::get_oracle(env, &key)
        .unwrap_or_else(|| panic_with_error!(env, OracleError::OracleNotConfigured));
    assert_with_error!(
        env,
        !oracle.has_aquarius_lp_source(),
        OracleError::SourceCountOutOfRange
    );
    validate_oracle_tolerance(env, &tolerance);
    oracle.tolerance = tolerance;
    let mut session = Session::new(env);
    engine::probe(&mut session, &key, &oracle);
    registry::commit(env, &key, &oracle);
}

#[cfg(test)]
#[path = "../tests/oracle/admin.rs"]
mod tests;
