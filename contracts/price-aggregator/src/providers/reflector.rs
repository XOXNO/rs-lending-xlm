//! Reads prices from Reflector oracle feeds in spot or TWAP mode. `attest`
//! checks a feed's quote base, decimals and resolution at admission. A TWAP
//! read takes the newest `records` observations of the answer and checks
//! their spacing, future timestamps and window coverage before averaging
//! them.

use common::errors::{GenericError, OracleError};
use common::oracle::observation::{
    is_future_at, try_normalize_positive_price, MIN_ORACLE_RESOLUTION_SECONDS,
};
use common::oracle::providers::reflector::{
    reflector_last_price, reflector_prices, to_reflector_asset, try_reflector_resolution,
    try_twap_mean_price, ReflectorAsset, ReflectorClient,
};
use common::types::{OracleReadMode, PriceKey, ReflectorFeedRef};
use common::validation::validate_twap_records;
use soroban_sdk::{assert_with_error, panic_with_error, Env, Symbol};

use crate::observation::OracleObservation;
use crate::session::Session;

/// Validates that `feed` is configured consistently with `decimals` and
/// `max_stale`: the quote base is denominated as `quote` requires, reported
/// decimals match `decimals`, and the resolution is at least
/// `MIN_ORACLE_RESOLUTION_SECONDS` and at most `max_stale`. For TWAP mode,
/// also checks that the oldest sample a read uses stays within `max_stale`
/// while the feed publishes on time; see [`twap_oldest_sample_age`]. Panics
/// with `InvalidOracleBase`, `InvalidOracleDecimals` or
/// `InvalidOracleResolution`.
///
/// `quote` is `None` for a bare feed and `Some` for the factor leg of a
/// `Scaled` source. See [`attest_base`] for the rule each case enforces.
pub(crate) fn attest(
    env: &Env,
    feed: &ReflectorFeedRef,
    decimals: u32,
    max_stale: u64,
    quote: Option<&PriceKey>,
) {
    let client = ReflectorClient::new(env, &feed.contract);
    attest_base(env, &client.base(), quote);
    assert_with_error!(
        env,
        client.decimals() == decimals,
        OracleError::InvalidOracleDecimals
    );
    let resolution = client.resolution();
    assert_with_error!(
        env,
        resolution >= MIN_ORACLE_RESOLUTION_SECONDS && u64::from(resolution) <= max_stale,
        OracleError::InvalidOracleResolution
    );
    if let OracleReadMode::Twap(records) = feed.read_mode {
        assert_with_error!(
            env,
            twap_oldest_sample_age(env, records, resolution) <= max_stale,
            OracleError::InvalidOracleResolution
        );
    }
}

/// Smallest span in seconds, newest sample to oldest, that a `Twap(records)`
/// read accepts at `resolution`.
fn twap_required_span(records: u32, resolution: u32) -> u64 {
    u64::from(records.saturating_sub(1)).saturating_mul(u64::from(resolution))
}

/// Largest age in seconds of the oldest sample a `Twap(records)` read uses
/// while the feed publishes one round per `resolution`.
///
/// A read uses the newest `records` rounds, so its oldest sample is
/// `records - 1` rounds behind the newest. The newest round is one
/// `resolution` old when the next round is due. The read carries the oldest
/// timestamp, so the staleness check sees `records * resolution`.
///
/// Panics with `GenericError::MathOverflow` on overflow, which two `u32`
/// inputs cannot reach in `u64`.
fn twap_oldest_sample_age(env: &Env, records: u32, resolution: u32) -> u64 {
    u64::from(records)
        .checked_mul(u64::from(resolution))
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
}

/// Panics with `InvalidOracleBase` unless a Reflector contract's quote base
/// matches how its price is consumed.
///
/// A bare feed (`quote` is `None`) is read as a USD price, so the contract
/// must quote in USD. A `Scaled` factor is multiplied by the price resolved
/// for `quote`, so a contract quoting in token `X` may only be scaled by
/// `Token(X)`; any other pairing multiplies prices in two different
/// currencies. A `Ref` quote has no on-chain asset identity and is always
/// rejected. A USD-quoting contract is rejected as a factor, because no
/// `Token` key prices USD.
fn attest_base(env: &Env, base: &ReflectorAsset, quote: Option<&PriceKey>) {
    let ok = match (base, quote) {
        (ReflectorAsset::Other(symbol), None) => *symbol == Symbol::new(env, "USD"),
        (ReflectorAsset::Stellar(base_asset), Some(PriceKey::Token(quote_asset))) => {
            base_asset == quote_asset
        }
        _ => false,
    };
    if !ok {
        panic_with_error!(env, OracleError::InvalidOracleBase);
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

/// Reads `feed`'s price and converts it from `decimals` to WAD, dispatching
/// to a spot read or a TWAP read depending on `feed.read_mode`. Returns
/// `None` if the read fails for either mode.
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

/// Reads the latest Reflector price for `feed.asset` and converts it from
/// `decimals` to a WAD `OracleObservation`. Returns `None` if the
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

/// Computes an arithmetic mean of the newest `records` Reflector
/// observations, despite the `Twap` name implying a time-weighted average.
/// The answer is newest first; observations after the first `records` are
/// ignored. Rejects a used history that is empty, future-dated, mis-ordered,
/// tightly spaced, or short of the window `records` implies. Returns the mean
/// with the timestamp of the oldest observation used.
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
    // The public contract answers with one round more than `records`. A
    // shorter answer has a skipped round or a young history; the span check
    // below proves coverage.
    let history = history.slice(..records.min(history.len()));

    let resolution = try_reflector_resolution(env, &feed.contract)
        .filter(|resolution| *resolution >= MIN_ORACLE_RESOLUTION_SECONDS)
        .ok_or(OracleError::InvalidOracleResolution)?;

    let newest_ts = history.first().map_or(0, |price_data| price_data.timestamp);
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

    // Minimum window coverage; the loop proves the ordering.
    if newest_ts.saturating_sub(oldest_ts) < twap_required_span(records, resolution) {
        return Err(OracleError::TwapInsufficientObservations);
    }

    let raw_price = try_twap_mean_price(&history).ok_or(OracleError::InvalidPrice)?;
    let price_wad =
        try_normalize_positive_price(raw_price, decimals).ok_or(OracleError::InvalidPrice)?;
    Ok(OracleObservation {
        price_wad,
        timestamp: oldest_ts,
    })
}
