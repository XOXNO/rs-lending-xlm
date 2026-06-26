//! Oracle price normalization, staleness, and future-timestamp guards.
//!
//! Owns the clock-skew and freshness constants shared by production price
//! resolution and the oracle config validators.

use crate::constants::MS_PER_SECOND;
use crate::errors::{GenericError, OracleError};
use crate::math::fp::Wad;
use soroban_sdk::{assert_with_error, panic_with_error, Env, U256};

// Max drift between the ledger clock and an oracle publication timestamp.
const MAX_FUTURE_SKEW_SECONDS: u64 = 60;

pub const MAX_TWAP_RECORDS: u32 = 12;

pub const MIN_PRICE_STALE_SECONDS: u64 = 60;
pub const MAX_PRICE_STALE_SECONDS: u64 = 86_400;

pub const MIN_ORACLE_RESOLUTION_SECONDS: u32 = 60;

pub const MIN_ORACLE_DECIMALS: u32 = 1;
pub const MAX_ORACLE_DECIMALS: u32 = 18;

pub fn normalize_positive_price(env: &Env, price: i128, decimals: u32) -> i128 {
    assert_with_error!(env, price > 0, OracleError::InvalidPrice);
    Wad::from_token(price, decimals).raw()
}

pub fn is_stale(now_secs: u64, feed_ts: u64, max_stale: u64) -> bool {
    now_secs > feed_ts && (now_secs - feed_ts) > max_stale
}

pub fn check_not_future_at(env: &Env, now_secs: u64, feed_ts: u64) {
    let max_future_ts = now_secs
        .checked_add(MAX_FUTURE_SKEW_SECONDS)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    assert_with_error!(env, feed_ts <= max_future_ts, OracleError::PriceFeedStale);
}

fn validate_timestamp(env: &Env, now_secs: u64, feed_ts: u64, max_stale: u64) {
    check_not_future_at(env, now_secs, feed_ts);
    assert_with_error!(
        env,
        !is_stale(now_secs, feed_ts, max_stale),
        OracleError::PriceFeedStale
    );
}

/// Normalizes a positive token-denominated price to WAD and checks each feed
/// timestamp for future skew and staleness.
pub fn validate_positive_price_timestamps(
    env: &Env,
    raw_price: i128,
    decimals: u32,
    now_secs: u64,
    feed_timestamps: &[u64],
    max_stale: u64,
) -> i128 {
    let price_wad = normalize_positive_price(env, raw_price, decimals);
    for ts in feed_timestamps {
        validate_timestamp(env, now_secs, *ts, max_stale);
    }
    price_wad
}

pub fn u256_to_i128(env: &Env, value: &U256) -> i128 {
    let Some(raw) = value.to_u128() else {
        panic_with_error!(env, GenericError::MathOverflow);
    };
    assert_with_error!(env, raw <= i128::MAX as u128, GenericError::MathOverflow);
    raw as i128
}

pub fn millis_to_seconds(timestamp_ms: u64) -> u64 {
    // `MS_PER_SECOND` is a nonzero constant, so this division cannot fail.
    timestamp_ms / MS_PER_SECOND
}

#[cfg(test)]
#[path = "../../tests/oracle/observation.rs"]
mod tests;
