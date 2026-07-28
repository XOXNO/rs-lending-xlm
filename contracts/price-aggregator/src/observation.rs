//! Provider observations normalized to WAD with timestamp guards.
//!
//! Always soft: rejected payloads become `None`. Hard path maps miss →
//! unreadable → [`crate::engine::force`] panics with the gate error.

use common::oracle::observation::{
    is_future_at, millis_to_seconds, try_normalize_positive_price, try_u256_to_i128,
};
use common::oracle::providers::redstone::RedStonePriceData;
use common::oracle::providers::reflector::ReflectorPriceData;
use soroban_sdk::Env;

/// Provider price observation consumed by the compose layer.
#[cfg_attr(feature = "certora", allow(dead_code))]
#[derive(Clone, Debug)]
pub(crate) struct OracleObservation {
    pub price_wad: i128,
    pub observed_at: u64,
    pub published_at: Option<u64>,
}

impl OracleObservation {
    /// Strictest known freshness timestamp.
    pub(crate) fn timestamp(&self) -> u64 {
        self.published_at
            .map_or(self.observed_at, |t| t.min(self.observed_at))
    }

    /// From a multi-feed adapter payload (RedStone/Xoxno wire ABI).
    /// Both provider timestamps must not sit in the future relative to
    /// the ledger clock `now_secs`. Rejected → `None`.
    pub(crate) fn from_multi_feed(
        env: &Env,
        now_secs: u64,
        price_data: &RedStonePriceData,
        decimals: u32,
    ) -> Option<Self> {
        let _ = env;
        let package_ts = millis_to_seconds(price_data.package_timestamp);
        let write_ts = millis_to_seconds(price_data.write_timestamp);
        if is_future_at(now_secs, package_ts) || is_future_at(now_secs, write_ts) {
            return None;
        }
        let raw_price = try_u256_to_i128(&price_data.price)?;
        Some(OracleObservation {
            price_wad: try_normalize_positive_price(raw_price, decimals)?,
            observed_at: write_ts,
            published_at: Some(package_ts),
        })
    }

    /// From a Reflector spot payload. Feed timestamp must not sit in the
    /// future relative to `now_secs`. Rejected → `None`.
    pub(crate) fn from_reflector(
        env: &Env,
        now_secs: u64,
        price_data: &ReflectorPriceData,
        decimals: u32,
    ) -> Option<Self> {
        let _ = env;
        if is_future_at(now_secs, price_data.timestamp) {
            return None;
        }
        Some(OracleObservation {
            price_wad: try_normalize_positive_price(price_data.price, decimals)?,
            observed_at: price_data.timestamp,
            published_at: None,
        })
    }
}

#[cfg(test)]
#[path = "../tests/oracle/observation.rs"]
mod tests;
