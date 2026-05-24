use common::constants::MS_PER_SECOND;
use common::errors::{GenericError, OracleError};
use common::math::fp::Wad;
use common::types::{OracleProviderKind, OracleReadMode};
use soroban_sdk::{panic_with_error, Env, U256};

// Max drift between the ledger clock and an oracle publication timestamp.
const MAX_FUTURE_SKEW_SECONDS: u64 = 60;

pub(crate) const MAX_TWAP_RECORDS: u32 = 12;

pub(crate) const MIN_PRICE_STALE_SECONDS: u64 = 60;
pub(crate) const MAX_PRICE_STALE_SECONDS: u64 = 86_400;

pub(crate) const MIN_ORACLE_RESOLUTION_SECONDS: u32 = 60;

pub(crate) const MIN_ORACLE_DECIMALS: u32 = 1;
pub(crate) const MAX_ORACLE_DECIMALS: u32 = 18;

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
    // Min of published/observed timestamps.
    pub(crate) fn timestamp(&self) -> u64 {
        self.published_at
            .map_or(self.observed_at, |t| t.min(self.observed_at))
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

// U256 to i128.
pub(crate) fn u256_to_i128(env: &Env, value: &U256) -> i128 {
    let Some(raw) = value.to_u128() else {
        panic_with_error!(env, GenericError::MathOverflow);
    };
    if raw > i128::MAX as u128 {
        panic_with_error!(env, GenericError::MathOverflow);
    }
    raw as i128
}

// MS to seconds.
pub(crate) fn millis_to_seconds(env: &Env, timestamp_ms: u64) -> u64 {
    timestamp_ms
        .checked_div(MS_PER_SECOND)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
}
