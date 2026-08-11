//! Converts raw provider price payloads (RedStone, Reflector) into normalized,
//! WAD-scaled observations, rejecting prices timestamped in the future or that
//! fail decimal normalization.

use common::oracle::observation::{
    is_future_at, millis_to_seconds, try_normalize_positive_price, try_u256_to_i128,
};
use common::oracle::providers::redstone::RedStonePriceData;
use common::oracle::providers::reflector::ReflectorPriceData;
/// A normalized price observation: a WAD-scaled price and its observation
/// timestamp in seconds.
#[cfg_attr(feature = "certora", allow(dead_code))]
#[derive(Clone, Debug)]
pub(crate) struct OracleObservation {
    pub price_wad: i128,
    pub timestamp: u64,
}

impl OracleObservation {
    /// Builds an observation from a RedStone multi-feed payload. Rejects the
    /// observation if its package or write timestamp exceeds
    /// `now_secs + MAX_FUTURE_SKEW_SECONDS`, if the price does not fit an
    /// `i128`, or if it fails decimal normalization. Uses the earlier of the
    /// write and package timestamps.
    pub(crate) fn from_multi_feed(
        now_secs: u64,
        price_data: &RedStonePriceData,
        decimals: u32,
    ) -> Option<Self> {
        let package_ts = millis_to_seconds(price_data.package_timestamp);
        let write_ts = millis_to_seconds(price_data.write_timestamp);
        if is_future_at(now_secs, package_ts) || is_future_at(now_secs, write_ts) {
            return None;
        }
        let raw_price = try_u256_to_i128(&price_data.price)?;
        Some(OracleObservation {
            price_wad: try_normalize_positive_price(raw_price, decimals)?,
            timestamp: write_ts.min(package_ts),
        })
    }

    /// Builds an observation from a Reflector price payload. Rejects the
    /// observation if its timestamp exceeds `now_secs + MAX_FUTURE_SKEW_SECONDS`
    /// or if the price fails decimal normalization.
    pub(crate) fn from_reflector(
        now_secs: u64,
        price_data: &ReflectorPriceData,
        decimals: u32,
    ) -> Option<Self> {
        if is_future_at(now_secs, price_data.timestamp) {
            return None;
        }
        Some(OracleObservation {
            price_wad: try_normalize_positive_price(price_data.price, decimals)?,
            timestamp: price_data.timestamp,
        })
    }
}

#[cfg(test)]
#[path = "../tests/oracle/observation.rs"]
mod tests;
