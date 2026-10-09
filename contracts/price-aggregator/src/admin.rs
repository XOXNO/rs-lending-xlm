//! Owner-only oracle configuration: register an oracle, tighten its sanity band,
//! or change its tolerance.
//!
//! `set_oracle` also attests the price sources and revalidates every oracle whose
//! composition depends on the changed key.

use common::errors::OracleError;
use common::oracle::providers::redstone::REDSTONE_DECIMALS;
use common::oracle::providers::reflector::ReflectorClient;
use common::oracle::providers::xoxno::XoxnoOracleAdapterClient;
use common::types::{
    AssetOracle, FeedSource, OracleTolerance, PriceKey, PriceSource, ProviderRef,
    MAX_RESOLUTION_DEPTH,
};
use common::validation::{
    validate_lp_sanity_band, validate_oracle_tolerance, validate_sanity_bounds,
    validate_single_source_sanity_band,
};
use soroban_sdk::{assert_with_error, panic_with_error, Env, Vec};

use crate::engine;
use crate::properties;
use crate::providers::{aquarius, reflector};
use crate::registry;
use crate::session::Session;
use crate::validation;

/// Attests every price source configured on `oracle`, dispatching feed, scaled-feed,
/// Aquarius LP, and Aquarius stable-LP sources to their respective provider
/// attestation routines.
fn attest_sources(env: &Env, key: &PriceKey, oracle: &AssetOracle) {
    for source in oracle.sources.iter() {
        match &source {
            PriceSource::Feed(feed) => attest_feed(env, feed, None),
            PriceSource::Scaled(scaled) => attest_feed(env, &scaled.factor, Some(&scaled.quote)),

            PriceSource::AquariusLp(lp) => aquarius::attest(env, key, oracle, lp, false),
            PriceSource::AquariusStableLp(lp) => aquarius::attest(env, key, oracle, lp, true),
        }
    }
}

/// Attests a single feed source by dispatching to the provider-specific attestation
/// routine identified by `feed.provider`.
fn attest_feed(env: &Env, feed: &FeedSource, quote: Option<&PriceKey>) {
    match &feed.provider {
        ProviderRef::Reflector(reflector) => {
            reflector::attest(env, reflector, feed.decimals, feed.max_stale_seconds, quote)
        }
        ProviderRef::RedStone(_) => assert_with_error!(
            env,
            feed.decimals == REDSTONE_DECIMALS,
            OracleError::InvalidOracleDecimals
        ),
        ProviderRef::Xoxno(xoxno_feed) => {
            assert_with_error!(
                env,
                ReflectorClient::new(env, &xoxno_feed.contract).decimals() == feed.decimals,
                OracleError::InvalidOracleDecimals
            );
            assert_with_error!(
                env,
                feed.max_stale_seconds
                    >= XoxnoOracleAdapterClient::new(env, &xoxno_feed.contract)
                        .max_submission_age_seconds(),
                OracleError::InvalidStalenessConfig
            );
        }
    }
}

/// Validates `oracle` and attests its sources, then probes it: a hard probe
/// for Aquarius LP oracles that panics on any unusable outcome, or a soft
/// probe otherwise that panics only on configuration-level failures. Stores
/// the oracle, revalidates dependents, and emits the registry event.
///
/// A replacement must keep the stored `asset_decimals`. Panics with
/// `OracleError::InvalidOracleDecimals` otherwise.
pub(crate) fn set_oracle(env: &Env, key: PriceKey, oracle: AssetOracle) {
    if let Some(stored) = registry::get_oracle(env, &key) {
        assert_with_error!(
            env,
            stored.asset_decimals == oracle.asset_decimals,
            OracleError::InvalidOracleDecimals
        );
    }
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

/// Revalidates every registered oracle whose composition transitively depends
/// on `changed`, panicking if any of them fails validation under the new
/// state.
///
/// Walks the reverse-dependency index upward from `changed`, one level per
/// composition step, so the cost follows the actual dependents rather than the
/// size of the append-only registry. `MAX_RESOLUTION_DEPTH` levels reach every
/// dependent: a key that sat further above `changed` would already have
/// exceeded the depth limit. The first call on a registry stored before the
/// index existed builds the index once.
///
/// Structural only: the walk reads the registry and never crosses a contract
/// boundary. A live re-probe costs one VM instantiation per provider call, and
/// three LP dependents can exceed the 40 MiB mainnet transaction memory budget.
/// The pool-kind check is not repeated: only `aquarius::attest` raises
/// `UnsupportedAquariusPool`, it ran when the dependent was admitted, and
/// `changed` cannot alter the pool kind.
fn revalidate_dependents(env: &Env, changed: &PriceKey) {
    registry::ensure_dependents_indexed(env);
    let mut visited = Vec::from_array(env, [changed.clone()]);
    let mut level = visited.clone();
    for _ in 0..MAX_RESOLUTION_DEPTH {
        let mut next = Vec::new(env);
        for key in level.iter() {
            for dependent in registry::dependents(env, &key).iter() {
                if visited.contains(&dependent) {
                    continue;
                }
                let oracle = registry::get_oracle(env, &dependent)
                    .unwrap_or_else(|| panic_with_error!(env, OracleError::OracleNotConfigured));
                validate_asset_oracle(env, &dependent, &oracle);
                visited.push_back(dependent.clone());
                next.push_back(dependent);
            }
        }
        level = next;
    }
}

/// Runs the full validation suite for `oracle` under `key`, covering sanity
/// bounds, source shape and count, asset decimals, composition depth,
/// staleness, smoothing, tolerance, the leg-spread budget, and source
/// independence, with the smoothing and tolerance checks waived for Aquarius
/// LP oracles. Panics if any check fails.
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
        validate_lp_sanity_band(
            env,
            oracle.min_sanity_price_wad,
            oracle.max_sanity_price_wad,
        );
    } else {
        // A pair cross-checks its band only when no single contract can move
        // the blended price further than the single-source cap allows.
        let exempt_from_band_cap = derived.second.as_ref().is_some_and(|second| {
            !validation::same_address_set(&derived.first.trust, &second.trust)
                && derived
                    .first
                    .shared_contracts_with(env, second)
                    .iter()
                    .all(|contract| {
                        validation::reach_within_single_source_cap(properties::contract_reach(
                            env,
                            &contract,
                            &oracle.sources,
                            &oracle.tolerance,
                        ))
                    })
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
        validation::leg_spread_budget(env, &derived.first, second);
        validation::independence(env, &derived.first, second, &oracle.independence);
    }
}

/// Narrows the sanity band of the oracle under `key`; panics with
/// `SanityBandMustTighten` if either bound widens. Revalidates and re-probes
/// before committing.
///
/// Governance calls this on the immediate `ORACLE_ROLE` path, so it only
/// narrows and repeated calls cannot walk the band. Widening goes through the
/// timelocked `ConfigureAssetOracle` (INV-AUTH-04).
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
/// oracles, validates the new tolerance and the oracle under it, re-probes the
/// oracle, commits the result to the registry, and revalidates its dependents.
/// Panics if the oracle is not configured, has an Aquarius LP source, or the
/// tolerance fails validation for the key or any dependent.
///
/// The tolerance bounds how far a contract shared by the key's legs can move
/// the blended price, which decides the key's own band-cap exemption and,
/// through the key's quote role, those of its dependents.
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
    validate_asset_oracle(env, &key, &oracle);
    let mut session = Session::new(env);
    engine::probe(&mut session, &key, &oracle);
    registry::commit(env, &key, &oracle);
    revalidate_dependents(env, &key);
}

#[cfg(test)]
#[path = "../tests/oracle/admin.rs"]
mod tests;
