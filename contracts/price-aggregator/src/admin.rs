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

fn attest_sources(env: &Env, key: &PriceKey, oracle: &AssetOracle) {
    for source in oracle.sources.iter() {
        match &source {
            PriceSource::Feed(feed) => attest_feed(env, feed),
            PriceSource::Scaled(scaled) => attest_feed(env, &scaled.factor),

            PriceSource::AquariusLp(lp) => aquarius::attest(env, key, oracle, lp),
            PriceSource::AquariusStableLp(lp) => aquarius::attest_stable(env, key, oracle, lp),
        }
    }
}

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
    let exempt_from_band_cap = oracle.has_aquarius_lp_source()
        || derived
            .second
            .as_ref()
            .is_some_and(|second| !derived.first.trusts_exactly_as(second));
    validate_single_source_sanity_band(
        env,
        exempt_from_band_cap,
        oracle.min_sanity_price_wad,
        oracle.max_sanity_price_wad,
    );
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

pub(crate) fn set_sanity_band(env: &Env, key: PriceKey, min_wad: i128, max_wad: i128) {
    let mut oracle = registry::get_oracle(env, &key)
        .unwrap_or_else(|| panic_with_error!(env, OracleError::OracleNotConfigured));

    assert_with_error!(
        env,
        min_wad < oracle.max_sanity_price_wad && max_wad > oracle.min_sanity_price_wad,
        OracleError::InvalidSanityBounds
    );
    oracle.min_sanity_price_wad = min_wad;
    oracle.max_sanity_price_wad = max_wad;
    validate_asset_oracle(env, &key, &oracle);

    let mut session = Session::new(env);
    engine::probe(&mut session, &key, &oracle);
    registry::commit(env, &key, &oracle);
}

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
