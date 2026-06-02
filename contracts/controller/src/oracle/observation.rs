//! Oracle observation construction, normalization, staleness, and
//! future-timestamp guards.
//!
//! Single place that turns raw Reflector/RedStone responses into the internal
//! `OracleObservation` shape, and owns the clock-skew and freshness constants
//! shared by production price resolution and the oracle config validators.

use common::constants::MS_PER_SECOND;
use common::errors::{GenericError, OracleError};
use common::math::fp::Wad;
use soroban_sdk::{assert_with_error, panic_with_error, Env, U256};

// Max drift between the ledger clock and an oracle publication timestamp.
const MAX_FUTURE_SKEW_SECONDS: u64 = 60;

pub(crate) const MAX_TWAP_RECORDS: u32 = 12;

pub(crate) const MIN_PRICE_STALE_SECONDS: u64 = 60;
pub(crate) const MAX_PRICE_STALE_SECONDS: u64 = 86_400;

pub(crate) const MIN_ORACLE_RESOLUTION_SECONDS: u32 = 60;

pub(crate) const MIN_ORACLE_DECIMALS: u32 = 1;
pub(crate) const MAX_ORACLE_DECIMALS: u32 = 18;

/// Internal representation of a single oracle price observation, used by the
/// provider consumption logic and compose layer.
#[cfg_attr(feature = "certora", allow(dead_code))] // Dead when certora stubs price paths.
#[derive(Clone, Debug)]
pub(crate) struct OracleObservation {
    pub price_wad: i128,
    pub observed_at: u64,
    pub published_at: Option<u64>,
}

impl OracleObservation {
    // Min of published/observed timestamps.
    pub(crate) fn timestamp(&self) -> u64 {
        self.published_at
            .map_or(self.observed_at, |t| t.min(self.observed_at))
    }
}

pub(crate) fn normalize_positive_price(env: &Env, price: i128, decimals: u32) -> i128 {
    assert_with_error!(env, price > 0, OracleError::InvalidPrice);
    Wad::from_token(price, decimals).raw()
}

pub(crate) fn is_stale(now_secs: u64, feed_ts: u64, max_stale: u64) -> bool {
    now_secs > feed_ts && (now_secs - feed_ts) > max_stale
}

pub(crate) fn check_not_future_at(env: &Env, now_secs: u64, feed_ts: u64) {
    let max_future_ts = now_secs
        .checked_add(MAX_FUTURE_SKEW_SECONDS)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    assert_with_error!(env, feed_ts <= max_future_ts, OracleError::PriceFeedStale);
}

pub(crate) fn validate_timestamp(env: &Env, now_secs: u64, feed_ts: u64, max_stale: u64) {
    check_not_future_at(env, now_secs, feed_ts);
    assert_with_error!(
        env,
        !is_stale(now_secs, feed_ts, max_stale),
        OracleError::PriceFeedStale
    );
}

pub(crate) fn u256_to_i128(env: &Env, value: &U256) -> i128 {
    let Some(raw) = value.to_u128() else {
        panic_with_error!(env, GenericError::MathOverflow);
    };
    assert_with_error!(env, raw <= i128::MAX as u128, GenericError::MathOverflow);
    raw as i128
}

pub(crate) fn millis_to_seconds(timestamp_ms: u64) -> u64 {
    // `MS_PER_SECOND` is a nonzero constant, so this division cannot fail.
    timestamp_ms / MS_PER_SECOND
}

/// Shared constructor used by both oracle providers after their provider-specific
/// validation (future-skew, positive price, staleness): final WAD normalization
/// + struct assembly.
pub(crate) fn build_observation(
    env: &Env,
    raw_price: i128,
    decimals: u32,
    observed_at: u64,
    published_at: Option<u64>,
) -> OracleObservation {
    OracleObservation {
        price_wad: normalize_positive_price(env, raw_price, decimals),
        observed_at,
        published_at,
    }
}
