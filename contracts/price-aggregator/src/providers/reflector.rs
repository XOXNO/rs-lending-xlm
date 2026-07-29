//! Reflector SEP-40 reads: spot or TWAP. No bulk API — one call per feed.
//!
//! Spot is soft: bad market data → `None`. TWAP maps history / observation /
//! price failures to `Result` and then `.ok()` at the entry so soft views see
//! unreadable and hard `force` panics `NoLastPrice` (not TWAP-specific codes).
//! Invalid TWAP record counts still trap via `validate_twap_records`.

use common::errors::OracleError;
use common::oracle::observation::{is_future_at, MIN_ORACLE_RESOLUTION_SECONDS};
use common::oracle::providers::reflector::{
    min_twap_observations, reflector_lastprice_call, reflector_prices_call, to_reflector_asset,
    try_reflector_resolution_call, try_twap_mean_price,
};
use common::types::{OracleReadMode, ReflectorFeedRef};
use common::validation::validate_twap_records;

use crate::observation::OracleObservation;
use crate::session::Session;

#[cfg(feature = "certora")]
pub(crate) use certora_read::read_reflector_source;

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

/// Spot or TWAP observation for `feed`, normalized to WAD.
pub(crate) fn read_reflector_source_impl(
    session: &mut Session,
    feed: &ReflectorFeedRef,
    decimals: u32,
) -> Option<OracleObservation> {
    match feed.read_mode {
        OracleReadMode::Spot => read_spot(session, feed, decimals),
        // TWAP config/history problems are miss-equivalent after `.ok()`.
        OracleReadMode::Twap(records) => read_twap(session, feed, decimals, records).ok(),
    }
}

fn read_spot(
    session: &Session,
    feed: &ReflectorFeedRef,
    decimals: u32,
) -> Option<OracleObservation> {
    let env = session.env();
    let now_secs = session.now_secs();
    let asset = to_reflector_asset(env, &feed.asset);
    let price_data = reflector_lastprice_call(env, &feed.contract, &asset)?;
    OracleObservation::from_reflector(env, now_secs, &price_data, decimals)
}

fn read_twap(
    session: &Session,
    feed: &ReflectorFeedRef,
    decimals: u32,
    records: u32,
) -> Result<OracleObservation, OracleError> {
    let env = session.env();
    let now_secs = session.now_secs();
    // Invalid record count is a config bug — traps via validate.
    validate_twap_records(env, records);

    let asset = to_reflector_asset(env, &feed.asset);
    let Some(history) = reflector_prices_call(env, &feed.contract, &asset, records) else {
        return Err(OracleError::ReflectorHistoryEmpty);
    };
    if history.is_empty() {
        return Err(OracleError::ReflectorHistoryEmpty);
    }
    if history.len() < min_twap_observations(records) {
        return Err(OracleError::TwapInsufficientObservations);
    }
    if history.len() > records {
        return Err(OracleError::TwapInsufficientObservations);
    }

    let resolution = try_reflector_resolution_call(env, &feed.contract)
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
        observed_at: oldest_ts,
        published_at: None,
    })
}
