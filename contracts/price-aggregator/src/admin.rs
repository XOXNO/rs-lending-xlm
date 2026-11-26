//! Owner-only oracle configuration: register an oracle, tighten its sanity band,
//! or change its tolerance.
//!
//! `set_oracle` also attests the price sources and revalidates every oracle whose
//! composition depends on the changed key.

use common::errors::OracleError;
use common::oracle::providers::chainlink::CHAINLINK_DECIMALS;
use common::oracle::providers::redstone::REDSTONE_DECIMALS;
use common::oracle::providers::reflector::ReflectorClient;
use common::oracle::providers::xoxno::XoxnoOracleAdapterClient;
use common::types::{AssetOracle, FeedSource, OracleTolerance, PriceKey, PriceSource, ProviderRef};
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

/// Attests every price source configured on `oracle`. Feed, scaled-feed, Aquarius LP, and
/// Aquarius stable-LP sources go to their respective provider attestation routines. A
/// Curve LP source is a no-op here, because the hard probe in `set_oracle` enforces its
/// pool binding.
fn attest_sources(env: &Env, key: &PriceKey, oracle: &AssetOracle) {
    for source in oracle.sources.iter() {
        match &source {
            PriceSource::Feed(feed) => attest_feed(env, feed, None),
            PriceSource::Scaled(scaled) => attest_feed(env, &scaled.factor, Some(&scaled.quote)),

            PriceSource::AquariusLp(lp) => aquarius::attest(env, key, oracle, lp, false),
            PriceSource::AquariusStableLp(lp) => aquarius::attest(env, key, oracle, lp, true),
            PriceSource::CurveStableLp(_) => {}
        }
    }
}

/// Attests a single feed source with the provider-specific attestation routine that
/// `feed.provider` identifies.
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
        ProviderRef::Chainlink(_) => assert_with_error!(
            env,
            feed.decimals == CHAINLINK_DECIMALS,
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

/// Validates `oracle`, attests its sources, and then probes it. LP (Aquarius or Curve)
/// oracles get a hard probe that panics on any unusable outcome. Other oracles get a soft
/// probe that panics only on configuration-level failures. Stores the oracle, revalidates
/// dependents, and emits the registry event.
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
    if oracle.has_lp_source() {
        engine::probe_priceable(&mut session, &key, &oracle);
    } else {
        engine::probe(&mut session, &key, &oracle);
    }

    registry::store_oracle(env, &key, &oracle);
    revalidate_dependents(env, &key);
    registry::emit(env, &key, &oracle);
}

/// Revalidates every oracle registered in the registry whose composition
/// transitively depends on `changed`, panicking if any of them fails
/// validation under the new state.
///
/// The check is structural only. The walk reads the registry and never crosses a contract
/// boundary. A live re-probe costs one VM instantiation per provider call, and three LP
/// dependents can exceed the 40 MiB mainnet transaction memory budget. The pool-kind
/// check is not repeated. Only `aquarius::attest` raises `UnsupportedAquariusPool`, it
/// ran when the dependent was admitted, and `changed` cannot alter the pool kind.
fn revalidate_dependents(env: &Env, changed: &PriceKey) {
    for candidate in registry::oracle_keys(env).iter() {
        if candidate == *changed || !depends_on(env, &candidate, changed, &mut Vec::new(env)) {
            continue;
        }
        let oracle = registry::get_oracle(env, &candidate)
            .unwrap_or_else(|| panic_with_error!(env, OracleError::OracleNotConfigured));
        validate_oracle(env, &candidate, &oracle, false);
    }
}

/// Returns whether `root`'s registered oracle composition depends, directly or
/// transitively, on `target`. The walk guards against cycles with `visiting`. It treats a
/// key already on the current path as a non-match.
fn depends_on(env: &Env, root: &PriceKey, target: &PriceKey, visiting: &mut Vec<PriceKey>) -> bool {
    if visiting.first_index_of(root).is_some() {
        return false;
    }
    visiting.push_back(root.clone());
    let found = registry::get_oracle(env, root).is_some_and(|oracle| {
        oracle.sources.iter().any(|source| {
            properties::dependencies(env, &source)
                .iter()
                .any(|dependency| {
                    dependency == *target || depends_on(env, &dependency, target, visiting)
                })
        })
    });
    visiting.pop_back();
    found
}

/// Runs the full validation suite for `oracle` under `key`. The suite covers sanity
/// bounds, source shape and count, asset decimals, composition depth, staleness,
/// smoothing, tolerance, and source independence. The smoothing and tolerance checks are
/// waived for LP oracles. Panics if any check fails. An `AllowShared` declaration must
/// match the shared set exactly.
pub(crate) fn validate_asset_oracle(env: &Env, key: &PriceKey, oracle: &AssetOracle) {
    validate_oracle(env, key, oracle, true);
}

/// Runs the checks of [`validate_asset_oracle`]. When `exact_independence` is false, a
/// stored `AllowShared` declaration can cover a shared set that has since shrunk.
fn validate_oracle(env: &Env, key: &PriceKey, oracle: &AssetOracle, exact_independence: bool) {
    let mut session = Session::new(env);
    session.push_key(key);
    let derived = properties::properties_of_config(&mut session, &oracle.sources);
    session.pop_key();

    validate_sanity_bounds(
        env,
        oracle.min_sanity_price_wad,
        oracle.max_sanity_price_wad,
    );
    let exempt_from_band_cap = derived
        .second
        .as_ref()
        .is_some_and(|second| !validation::same_address_set(&derived.first.trust, &second.trust));
    if oracle.has_lp_source() {
        // Sole-source by construction, so the band is the only backstop.
        validate_lp_sanity_band(
            env,
            oracle.min_sanity_price_wad,
            oracle.max_sanity_price_wad,
        );
    } else {
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
        // A provider on both legs of an exempt pair moves the price up to its
        // factor range times the upper tolerance ratio: the legs only have to
        // agree within that ratio, and the price is their midpoint. The factor
        // range keeps the single-source cap to bound that reach.
        if let (PriceSource::Scaled(scaled), Some(second)) = (&source, derived.second.as_ref()) {
            let provider = scaled.factor.provider.contract();
            let shared = derived.first.trust.contains(provider) && second.trust.contains(provider);
            validate_single_source_sanity_band(
                env,
                !(exempt_from_band_cap && shared),
                scaled.min_factor_wad,
                scaled.max_factor_wad,
            );
        }
    }

    let has_lp = oracle.sources.iter().any(|source| source.is_lp());
    if has_lp && oracle.sources.len() != 1 {
        panic_with_error!(env, OracleError::SourceCountOutOfRange);
    }

    validation::composition_depth(env, &derived.first);
    if let Some(second) = derived.second.as_ref() {
        validation::composition_depth(env, second);
    }
    validation::staleness_envelope(env, oracle.max_price_stale_seconds, &derived.combined());
    if !oracle.has_lp_source() {
        validation::smoothing(env, &derived.first, derived.second.as_ref());
        validate_oracle_tolerance(env, &oracle.tolerance);
    }
    if let Some(second) = derived.second.as_ref() {
        validation::independence(
            env,
            &derived.first,
            second,
            &oracle.independence,
            exact_independence,
        );
    }
}

/// Narrows the sanity band of the oracle under `key`. Panics with `SanityBandMustTighten`
/// if either bound widens. Revalidates and re-probes before committing.
///
/// Governance calls this on the immediate `ORACLE_ROLE` path, so it only narrows.
/// Repeated calls cannot widen the band. Widening goes through the timelocked
/// `ConfigureAssetOracle` (INV-AUTH-04).
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
    validate_oracle(env, &key, &oracle, false);

    let mut session = Session::new(env);
    engine::probe(&mut session, &key, &oracle);
    registry::commit(env, &key, &oracle);
}

/// Updates the tolerance of the oracle registered under `key`. Rejects LP
/// (Aquarius or Curve) oracles, validates the new tolerance, re-probes the
/// oracle, and commits the result to the registry. Panics if the oracle is not
/// configured, has an LP source, or the tolerance fails validation.
pub(crate) fn set_tolerance(env: &Env, key: PriceKey, tolerance: OracleTolerance) {
    let mut oracle = registry::get_oracle(env, &key)
        .unwrap_or_else(|| panic_with_error!(env, OracleError::OracleNotConfigured));
    assert_with_error!(
        env,
        !oracle.has_lp_source(),
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
