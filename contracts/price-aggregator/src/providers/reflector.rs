use common::errors::OracleError;
use common::oracle::observation::{is_future_at, MIN_ORACLE_RESOLUTION_SECONDS};
use common::oracle::providers::reflector::{
    min_twap_observations, reflector_base, reflector_decimals, reflector_last_price,
    reflector_prices, reflector_resolution, to_reflector_asset, try_reflector_resolution,
    try_twap_mean_price, ReflectorAsset,
};
use common::types::{OracleReadMode, ReflectorFeedRef};
use common::validation::validate_twap_records;
use soroban_sdk::{assert_with_error, panic_with_error, Env, Symbol};

use crate::observation::OracleObservation;
use crate::session::Session;

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
    let price_data = reflector_last_price(env, &feed.contract, &asset)?;
    OracleObservation::from_reflector(now_secs, &price_data, decimals)
}

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
    if history.len() < min_twap_observations(records) {
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
