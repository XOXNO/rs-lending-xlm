//! Reads prices from Reflector oracle feeds in either spot or TWAP mode,
//! validating feed configuration (quote asset, decimals, resolution) and,
//! for TWAP, the spacing, staleness, and count of the underlying
//! observations before averaging them.

use common::errors::OracleError;
use common::oracle::observation::{is_future_at, MIN_ORACLE_RESOLUTION_SECONDS};
use common::oracle::providers::reflector::{
    reflector_base, reflector_decimals, reflector_last_price, reflector_prices,
    reflector_resolution, to_reflector_asset, try_reflector_resolution, try_twap_mean_price,
    ReflectorAsset,
};
use common::types::{OracleReadMode, ReflectorFeedRef};
use common::validation::validate_twap_records;
use soroban_sdk::{assert_with_error, panic_with_error, Env, Symbol};

use crate::observation::OracleObservation;
use crate::session::Session;

/// Validates that `feed` is configured consistently with `decimals` and
/// `max_stale`: the quote base is USD, reported decimals match `decimals`,
/// and the resolution is at least `MIN_ORACLE_RESOLUTION_SECONDS` and at
/// most `max_stale`. For TWAP mode, also checks that the span covered by
/// the requested record count does not exceed `max_stale`. Panics if any
/// check fails.
pub(crate) fn attest(env: &Env, feed: &ReflectorFeedRef, decimals: u32, max_stale: u64) {
    match reflector_base(env, &feed.contract) {
        ReflectorAsset::Other(symbol) if symbol == Symbol::new(env, "USD") => {}
        _ => panic_with_error!(env, OracleError::InvalidOracleBase),
    }
    assert_with_error!(
        env,
        reflector_decimals(env, &feed.contract) == decimals,
        OracleError::InvalidOracleDecimals
    );
    let resolution = reflector_resolution(env, &feed.contract);
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

#[cfg(feature = "certora")]
pub(crate) use certora_read::read_reflector_source;

/// Wraps `read_reflector_source_impl` behind a CVLR summary so the Certora
/// prover can substitute its own model of the call during formal
/// verification.
#[cfg(feature = "certora")]
mod certora_read {
    use super::*;
    cvlr_soroban_macros::apply_summary!(
        crate::spec::summaries::read_reflector_source_summary,
        pub(crate) fn read_reflector_source(
            session: &mut Session,
            feed: &ReflectorFeedRef,
            decimals: u32,
        ) -> Option<OracleObservation> {
            super::read_reflector_source_impl(session, feed, decimals)
        }
    );
}

#[cfg(not(feature = "certora"))]
pub(crate) use read_reflector_source_impl as read_reflector_source;

/// Reads `feed`'s price scaled to `decimals`, dispatching to a spot read or
/// a TWAP read depending on `feed.read_mode`. Returns `None` if the read
/// fails for either mode.
pub(crate) fn read_reflector_source_impl(
    session: &mut Session,
    feed: &ReflectorFeedRef,
    decimals: u32,
) -> Option<OracleObservation> {
    match feed.read_mode {
        OracleReadMode::Spot => read_spot(session, feed, decimals),

        OracleReadMode::Twap(records) => read_twap(session, feed, decimals, records).ok(),
    }
}

/// Reads the latest Reflector price for `feed.asset` and converts it to an
/// `OracleObservation` scaled to `decimals`. Returns `None` if the
/// underlying price cannot be read or the conversion fails.
fn read_spot(
    session: &Session,
    feed: &ReflectorFeedRef,
    decimals: u32,
) -> Option<OracleObservation> {
    let env = session.env();
    let now_secs = session.now_secs();
    let asset = to_reflector_asset(env, &feed.asset);
    let price_data = reflector_last_price(env, &feed.contract, &asset)?;
    OracleObservation::from_reflector(now_secs, &price_data, decimals)
}

/// Computes an arithmetic mean of up to `records` historical Reflector
/// observations, despite the `Twap` mode name implying a time-weighted
/// average. Validates that the history is non-empty, has between the
/// minimum observation count and `records + 1` entries, has no timestamp
/// beyond `now + MAX_FUTURE_SKEW_SECONDS`, and has consecutive timestamps
/// spaced at least one resolution period apart. Returns the mean price
/// paired with the oldest observation's timestamp, or an error if
/// validation or computation fails.
fn read_twap(
    session: &Session,
    feed: &ReflectorFeedRef,
    decimals: u32,
    records: u32,
) -> Result<OracleObservation, OracleError> {
    let env = session.env();
    let now_secs = session.now_secs();

    validate_twap_records(env, records);

    let asset = to_reflector_asset(env, &feed.asset);
    let Some(history) = reflector_prices(env, &feed.contract, &asset, records) else {
        return Err(OracleError::ReflectorHistoryEmpty);
    };
    if history.is_empty() {
        return Err(OracleError::ReflectorHistoryEmpty);
    }
    // A TWAP must use its full configured window.
    if history.len() < records {
        return Err(OracleError::TwapInsufficientObservations);
    }
    if history.len() > records.saturating_add(1) {
        return Err(OracleError::TwapInsufficientObservations);
    }

    let resolution = try_reflector_resolution(env, &feed.contract)
        .filter(|resolution| *resolution >= MIN_ORACLE_RESOLUTION_SECONDS)
        .ok_or(OracleError::InvalidOracleResolution)?;

    let mut oldest_ts = u64::MAX;
    let mut previous_ts = None;
    for price_data in history.iter() {
        if is_future_at(now_secs, price_data.timestamp) {
            return Err(OracleError::PriceFeedStale);
        }
        if previous_ts.is_some_and(|newer: u64| {
            newer
                .checked_sub(price_data.timestamp)
                .is_none_or(|spacing| spacing < u64::from(resolution))
        }) {
            return Err(OracleError::TwapInsufficientObservations);
        }
        previous_ts = Some(price_data.timestamp);
        oldest_ts = price_data.timestamp;
    }

    let raw_price = try_twap_mean_price(&history).ok_or(OracleError::InvalidPrice)?;
    let price_wad = common::oracle::observation::try_normalize_positive_price(raw_price, decimals)
        .ok_or(OracleError::InvalidPrice)?;
    Ok(OracleObservation {
        price_wad,
        timestamp: oldest_ts,
    })
}
