use common::errors::{GenericError, OracleError};
use common::fp::Wad;
use common::types::{OracleProviderKind, OracleReadMode};
use soroban_sdk::{panic_with_error, Env};

const MAX_FUTURE_SKEW_SECONDS: u64 = 60;

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct OracleObservation {
    pub price_wad: i128,
    pub raw_price: i128,
    pub raw_decimals: u32,
    pub observed_at: u64,
    pub published_at: Option<u64>,
    pub provider: OracleProviderKind,
    pub read_mode: OracleReadMode,
}

impl OracleObservation {
    pub(crate) fn timestamp(&self) -> u64 {
        match self.published_at {
            Some(published_at) if published_at < self.observed_at => published_at,
            _ => self.observed_at,
        }
    }
}

pub(crate) fn normalize_positive_price(env: &Env, price: i128, decimals: u32) -> i128 {
    if price <= 0 {
        panic_with_error!(env, OracleError::InvalidPrice);
    }
    Wad::from_token(price, decimals).raw()
}

pub(crate) fn is_stale(now_secs: u64, feed_ts: u64, max_stale: u64) -> bool {
    now_secs > feed_ts && (now_secs - feed_ts) > max_stale
}

pub(crate) fn check_not_future_at(env: &Env, now_secs: u64, feed_ts: u64) {
    let max_future_ts = now_secs
        .checked_add(MAX_FUTURE_SKEW_SECONDS)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    if feed_ts > max_future_ts {
        panic_with_error!(env, OracleError::PriceFeedStale);
    }
}

pub(crate) fn validate_timestamp(env: &Env, now_secs: u64, feed_ts: u64, max_stale: u64) {
    check_not_future_at(env, now_secs, feed_ts);
    if is_stale(now_secs, feed_ts, max_stale) {
        panic_with_error!(env, OracleError::PriceFeedStale);
    }
}
