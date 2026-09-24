//! Reads prices from Reflector oracle feeds in spot or TWAP mode. `attest`
//! checks a feed's quote base, decimals and resolution at admission. A TWAP
//! read checks the spacing, future timestamps and window coverage of its
//! observations before averaging them.

use common::errors::OracleError;
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
/// also checks that the span covered by the requested record count does not
/// exceed `max_stale`. Panics with `InvalidOracleBase`,
/// `InvalidOracleDecimals` or `InvalidOracleResolution`.
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
            twap_required_span(records, resolution) <= max_stale,
            OracleError::InvalidOracleResolution
        );
    }
}

/// Seconds a `Twap(records)` window spans at `resolution`.
fn twap_required_span(records: u32, resolution: u32) -> u64 {
    u64::from(records.saturating_sub(1)).saturating_mul(u64::from(resolution))
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

/// Computes an arithmetic mean of the Reflector observations, despite the
/// `Twap` name implying a time-weighted average. Rejects a history that is
/// empty, oversized, future-dated, mis-ordered, tightly spaced, or short of
/// the window `records` implies. Returns the mean with the oldest timestamp.
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
    // A skipped round is normal; the span check below proves coverage.
    if history.len() > records.saturating_add(1) {
        return Err(OracleError::TwapInsufficientObservations);
    }

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

    // Same span `attest` checks at admission; the loop proves the ordering.
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
