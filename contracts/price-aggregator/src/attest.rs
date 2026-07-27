//! Configure-time attestation: every provider fact the config *asserts* is
//! checked against what the provider itself *publishes*, once, before the
//! config is stored.
//!
//! # Why this exists
//!
//! The composable model deliberately carries `decimals` and the staleness
//! window as **config data** rather than reading them from the provider on
//! every price call. That is the right trade for the read path — a portfolio
//! price touches many feeds, and re-reading immutable metadata per feed per
//! call is pure overhead.
//!
//! It is the wrong trade for the *write* path. Config data is whatever the
//! proposer typed. A config that declares `decimals: 14` against a contract
//! publishing 7 does not fail: it reads a live, fresh, entirely plausible
//! number that is wrong by 10^7. Nothing downstream can catch that — the
//! staleness gate sees a fresh timestamp, the sanity band was set by the same
//! proposer to match the wrong number, and a single-source market has nothing
//! to disagree with it.
//!
//! So the declaration is verified here, at configure time, where it costs one
//! call per source per listing and a mismatch is a rejection instead of a
//! silently mispriced market. The read path stays cheap and keeps trusting the
//! stored config, because storing a false one is now impossible.
//!
//! Immutability is the assumption that makes this sound: a provider that
//! changes its published decimals after listing invalidates the attestation.
//! Reflector's `decimals`/`resolution` and the RedStone wire format are fixed
//! for the life of a deployment. A migration to a re-scaled provider is a
//! re-listing, not a silent upgrade.

use common::errors::{GenericError, OracleError};
use common::oracle::observation::MIN_ORACLE_RESOLUTION_SECONDS;
use common::oracle::providers::redstone::{xoxno_max_submission_age_call, REDSTONE_DECIMALS};
use common::oracle::providers::reflector::{
    reflector_base_call, reflector_decimals_call, reflector_resolution_call, ReflectorAsset,
};
use common::types::{AssetOracle, FeedSource, PriceSource, ProviderKind, ProviderRef};
use soroban_sdk::{assert_with_error, panic_with_error, Env, Symbol};

/// Attests every direct feed reachable from `oracle`'s own source list.
///
/// Nested keys (a `Scaled` quote, an `LpShare` leg) are **not** walked: each was
/// attested when *it* was configured, and re-attesting here would re-read the
/// whole dependency graph on every edit of any config that references it.
pub(crate) fn attest_sources(env: &Env, oracle: &AssetOracle) {
    for source in oracle.sources.iter() {
        match &source {
            PriceSource::Feed(feed) => attest_feed(env, feed),
            PriceSource::Scaled(scaled) => attest_feed(env, &scaled.factor),
            // Rejected by `validate_source_shape` before this runs.
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
            // The RedStone payload encodes prices at a fixed 8 decimals in the
            // wire format itself; the adapter publishes no `decimals()` to ask.
            // Declaring anything else misreads every price it returns.
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
            // Rejected by `validate_feed_shape` before this runs; repeated here
            // so the refusal does not depend on call order.
            ProviderKind::Reflector => {
                panic_with_error!(env, GenericError::InvalidExchangeSrc)
            }
        },
    }
}

/// Reflector publishes `base`, `decimals`, and `resolution`. All three are read
/// and matched against the config.
fn attest_reflector(env: &Env, contract: &soroban_sdk::Address, decimals: u32, max_stale: u64) {
    // A non-USD base means every number this contract returns is denominated in
    // something else. The composable model has no implicit quote hop — a quoted
    // Reflector deployment is expressed as a `Scaled` source naming its quote
    // key explicitly — so a quoted base reaching a direct `Feed` is a config
    // that would price the asset in the wrong unit.
    match reflector_base_call(env, contract) {
        ReflectorAsset::Other(symbol) if symbol == Symbol::new(env, "USD") => {}
        _ => panic_with_error!(env, OracleError::InvalidOracleBase),
    }

    assert_with_error!(
        env,
        reflector_decimals_call(env, contract) == decimals,
        OracleError::InvalidOracleDecimals
    );

    // `resolution` is the publish period. Below the floor the feed is not a
    // real time series; above the consumer's own staleness ceiling, a feed that
    // is behaving perfectly still reads as stale on every call, which bricks
    // the market rather than protecting it.
    let resolution = reflector_resolution_call(env, contract);
    assert_with_error!(
        env,
        resolution >= MIN_ORACLE_RESOLUTION_SECONDS && u64::from(resolution) <= max_stale,
        OracleError::InvalidOracleResolution
    );
}

/// The XOXNO adapter mirrors Reflector's `decimals()` and additionally
/// publishes the inclusion window it accepts submissions within.
fn attest_xoxno(env: &Env, contract: &soroban_sdk::Address, decimals: u32, max_stale: u64) {
    assert_with_error!(
        env,
        reflector_decimals_call(env, contract) == decimals,
        OracleError::InvalidOracleDecimals
    );

    // A consumer window tighter than the adapter's own inclusion window is a
    // liveness hazard, not a safety one: the adapter may legitimately publish a
    // price stamped up to `max_submission_age` old, and we would reject it. One
    // lagging signer then halts every risk read on the asset.
    let adapter_window = xoxno_max_submission_age_call(env, contract);
    assert_with_error!(
        env,
        max_stale >= adapter_window,
        OracleError::InvalidStalenessConfig
    );
}
