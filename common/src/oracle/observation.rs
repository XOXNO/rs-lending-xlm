use crate::constants::{MS_PER_SECOND, WAD_DECIMALS};
use crate::errors::{GenericError, OracleError};
use soroban_sdk::{assert_with_error, panic_with_error, Env, U256};

pub const MAX_FUTURE_SKEW_SECONDS: u64 = 60;

pub const MAX_TWAP_RECORDS: u32 = 12;

pub const MIN_PRICE_STALE_SECONDS: u64 = 60;
pub const MAX_PRICE_STALE_SECONDS: u64 = 86_400;

pub const MIN_ORACLE_RESOLUTION_SECONDS: u32 = 60;

pub const MIN_ORACLE_DECIMALS: u32 = 1;
pub const MAX_ORACLE_DECIMALS: u32 = 18;

pub const MAX_SINGLE_SOURCE_SANITY_BAND_BPS: i128 = 1_000;

pub fn try_normalize_positive_price(price: i128, decimals: u32) -> Option<i128> {
    if price <= 0 || decimals > WAD_DECIMALS {
        return None;
    }
    let factor = 10i128.checked_pow(WAD_DECIMALS - decimals)?;
    price.checked_mul(factor)
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

pub fn is_future_at(now_secs: u64, feed_ts: u64) -> bool {
    match now_secs.checked_add(MAX_FUTURE_SKEW_SECONDS) {
        Some(max_future_ts) => feed_ts > max_future_ts,
        None => false,
    }
}

pub fn u256_to_i128(env: &Env, value: &U256) -> i128 {
    let Some(raw) = value.to_u128() else {
        panic_with_error!(env, GenericError::MathOverflow);
    };
    assert_with_error!(env, raw <= i128::MAX as u128, GenericError::MathOverflow);
    raw as i128
}

pub fn try_u256_to_i128(value: &U256) -> Option<i128> {
    let raw = value.to_u128()?;
    (raw <= i128::MAX as u128).then_some(raw as i128)
}

pub fn millis_to_seconds(timestamp_ms: u64) -> u64 {
    timestamp_ms / MS_PER_SECOND
}

#[cfg(test)]
#[path = "../../tests/oracle/observation.rs"]
mod tests;
