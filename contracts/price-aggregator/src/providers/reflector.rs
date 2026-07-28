//! Reflector SEP-40: spot or TWAP. No bulk API — one call per feed.
//! Always soft: bad market data → `None` (hard path maps via force).

use common::errors::OracleError;
use common::oracle::observation::is_future_at;
use common::oracle::providers::reflector::{
    min_twap_observations, reflector_lastprice_call, reflector_prices_call, to_reflector_asset,
    try_twap_mean_price,
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

pub(crate) fn read_reflector_source_impl(
    session: &mut Session,
    feed: &ReflectorFeedRef,
    decimals: u32,
) -> Option<OracleObservation> {
    match feed.read_mode {
        OracleReadMode::Spot => read_spot(session, feed, decimals),
        // TWAP config/history problems are miss-equivalent: soft views show
        // unreadable; hard force panics NoLastPrice (not TWAP-specific codes).
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
    // Invalid record count is a config bug — still traps (validate panics).
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

    let mut oldest_ts = u64::MAX;
    for price_data in history.iter() {
        if is_future_at(now_secs, price_data.timestamp) {
            return Err(OracleError::PriceFeedStale);
        }
        if price_data.timestamp < oldest_ts {
            oldest_ts = price_data.timestamp;
        }
    }

    let raw_price = try_twap_mean_price(&history).ok_or(OracleError::InvalidPrice)?;
    let price_wad =
        common::oracle::observation::try_normalize_positive_price(raw_price, decimals)
            .ok_or(OracleError::InvalidPrice)?;
    Ok(OracleObservation {
        price_wad,
        observed_at: oldest_ts,
        published_at: None,
    })
}
