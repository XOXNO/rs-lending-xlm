use common::constants::{TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};
use common::errors::{GenericError, OracleError};
use common::oracle::observation::MIN_ORACLE_RESOLUTION_SECONDS;
use common::oracle::policy;
use common::oracle::providers::aquarius::{
    aquarius_get_tokens_call, aquarius_plane_of_pool_call, aquarius_plane_reserves_call,
    aquarius_share_id_call,
};
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
use soroban_sdk::token::TokenClient;
use soroban_sdk::{
    assert_with_error, contractevent, contracttype, panic_with_error, Env, Symbol, Vec,
};

use crate::engine;
use crate::properties;
use crate::session::Session;

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
    let mut keys = oracle_keys(env);
    if keys.first_index_of(key).is_none() {
        keys.push_back(key.clone());
        store_oracle_keys(env, &keys);
    }
}

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

fn attest_sources(env: &Env, oracle: &AssetOracle) {
    for source in oracle.sources.iter() {
        match &source {
            PriceSource::Feed(feed) => attest_feed(env, feed),
            PriceSource::Scaled(scaled) => attest_feed(env, &scaled.factor),

            PriceSource::LpShare(_) => {}
        }
    }
}

/// Binds an LpShare source to its real pool at listing time (config trust is the
/// only attack surface once the math is sound): the pool must be a constant-product
/// ("standard") pool, the `plane` must be the pool's own reserve mirror, the priced
/// key must be the pool's LP share token, and the configured reserve decimals must
/// match the pool's actual reserve tokens. Cross-contract reads run once, at
/// governance listing.
fn attest_lp_binding(env: &Env, key: &PriceKey, oracle: &AssetOracle) {
    for source in oracle.sources.iter() {
        let PriceSource::LpShare(lp) = &source else {
            continue;
        };
        assert_with_error!(
            env,
            aquarius_plane_of_pool_call(env, &lp.pool).as_ref() == Some(&lp.plane),
            OracleError::InvalidOracleBase
        );
        // Structural, not transient: a non-"standard" pool exposes bucketed or
        // otherwise differently-shaped reserves that this formula must never
        // price, and no later read can fix it. Rejecting here (rather than only
        // at read time) keeps a mislisted pool from being stored as a market
        // that is silently dead on first price.
        assert_with_error!(
            env,
            aquarius_plane_reserves_call(env, &lp.plane, &lp.pool).is_some(),
            OracleError::UnsupportedPoolKind
        );
        let share = aquarius_share_id_call(env, &lp.pool)
            .unwrap_or_else(|| panic_with_error!(env, OracleError::InvalidOracleBase));
        assert_with_error!(
            env,
            *key == PriceKey::Token(share.clone()),
            OracleError::InvalidOracleBase
        );
        // Both decimals fields describe the same share token, and each scales the
        // final price by a power of ten: `share_decimals` divides the supply in
        // `fair_lp_price_wad`, `asset_decimals` is what consumers convert with.
        // A typo in either is a silent 10^k misprice, so pin both to the token.
        let share_decimals = TokenClient::new(env, &share).decimals();
        assert_with_error!(
            env,
            share_decimals == lp.share_decimals && share_decimals == oracle.asset_decimals,
            OracleError::InvalidOracleDecimals
        );
        let tokens = aquarius_get_tokens_call(env, &lp.pool)
            .filter(|tokens| tokens.len() == 2)
            .unwrap_or_else(|| panic_with_error!(env, OracleError::InvalidOracleBase));
        assert_with_error!(
            env,
            TokenClient::new(env, &tokens.get_unchecked(0)).decimals() == lp.reserve_a_decimals
                && TokenClient::new(env, &tokens.get_unchecked(1)).decimals()
                    == lp.reserve_b_decimals,
            OracleError::InvalidOracleDecimals
        );
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

pub(crate) fn set_oracle(env: &Env, key: PriceKey, oracle: AssetOracle) {
    validate_asset_oracle(env, &key, &oracle);
    attest_sources(env, &oracle);
    attest_lp_binding(env, &key, &oracle);
    let mut session = Session::new(env);
    engine::probe(&mut session, &key, &oracle);

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

/// Two sources whose resolved trust sets differ — each leg knows something the
/// other does not, so their agreement is evidence.
fn independent_dual(env: &Env, key: &PriceKey, oracle: &AssetOracle) -> bool {
    if !oracle.is_dual() {
        return false;
    }
    let mut session = Session::new(env);
    session.push_key(key);
    let derived = properties::properties_of_config(&mut session, &oracle.sources);
    session.pop_key();
    derived
        .second
        .as_ref()
        .is_some_and(|second| !derived.first.trusts_exactly_as(second))
}

pub(crate) fn validate_asset_oracle(env: &Env, key: &PriceKey, oracle: &AssetOracle) {
    validate_sanity_bounds(
        env,
        oracle.min_sanity_price_wad,
        oracle.max_sanity_price_wad,
    );
    // The wide-band exemption is earned by an actual cross-check, not by arity:
    // two legs that trust exactly the same contracts always move together, so
    // their deviation band proves nothing and the sanity band is again the only
    // guard. `derived` is computed below for the same oracle, so the exemption is
    // decided from the resolved trust sets rather than `sources.len()`.
    let exempt_from_band_cap = oracle.has_lp_source() || independent_dual(env, key, oracle);
    validate_single_source_sanity_band(
        env,
        exempt_from_band_cap,
        oracle.min_sanity_price_wad,
        oracle.max_sanity_price_wad,
    );
    policy::validate_asset_decimals(env, key, oracle.asset_decimals);

    for source in oracle.sources.iter() {
        policy::validate_source_shape(env, &source);
    }

    // An LP share is priced standalone from pool reserves; blending it with a
    // second leg through the tolerance band is meaningless, so an LP source must
    // be the oracle's only source.
    let has_lp = oracle
        .sources
        .iter()
        .any(|source| matches!(source, PriceSource::LpShare(_)));
    if has_lp && oracle.sources.len() != 1 {
        panic_with_error!(env, OracleError::SourceCountOutOfRange);
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
    // The spot-only gate asks for a time window over a manipulable market read.
    // An LP share has no window of its own, but it does not need one: the reserve
    // ratio — the only part a swap can move — cancels out of 2*sqrt(Va*Vb)/S, and
    // both underlying prices come from oracles that passed this same gate when
    // they were listed. An LpShare is also always the sole source, so applying the
    // gate here would make every LP oracle unlistable.
    if !oracle.has_lp_source() {
        policy::validate_smoothing(env, &derived.first, derived.second.as_ref());
    }

    // The tolerance band is the primary/anchor agreement check, read only on the
    // two-leg blend path. An LpShare is always the sole source, so no band exists
    // to honour and requiring a valid one would force config authors to invent a
    // number that implies a cross-check the oracle does not perform.
    if !oracle.has_lp_source() {
        validate_oracle_tolerance(env, &oracle.tolerance);
    }
    if let Some(second) = derived.second.as_ref() {
        policy::validate_independence(env, &derived.first, second, &oracle.independence);
    }
}

pub(crate) fn set_sanity_band(env: &Env, key: PriceKey, min_wad: i128, max_wad: i128) {
    let mut oracle = get_oracle(env, &key)
        .unwrap_or_else(|| panic_with_error!(env, OracleError::OracleNotConfigured));

    validate_sanity_bounds(env, min_wad, max_wad);
    validate_single_source_sanity_band(env, oracle.is_dual() || oracle.has_lp_source(), min_wad, max_wad);
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

pub(crate) fn set_tolerance(env: &Env, key: PriceKey, tolerance: OracleTolerance) {
    let mut oracle = get_oracle(env, &key)
        .unwrap_or_else(|| panic_with_error!(env, OracleError::OracleNotConfigured));
    // Refuse rather than silently store a band an LP oracle will never read.
    assert_with_error!(
        env,
        !oracle.has_lp_source(),
        OracleError::SourceCountOutOfRange
    );
    validate_oracle_tolerance(env, &tolerance);
    oracle.tolerance = tolerance;
    let mut session = Session::new(env);
    engine::probe(&mut session, &key, &oracle);
    commit(env, &key, &oracle);
}

#[cfg(test)]
#[path = "../tests/oracle/registry.rs"]
mod tests;
