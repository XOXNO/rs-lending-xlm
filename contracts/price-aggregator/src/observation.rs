//! Provider observations normalized to WAD with timestamp guards.

use common::oracle::observation::{
    check_not_future_at, is_future_at, millis_to_seconds, normalize_positive_price,
    try_normalize_positive_price, try_u256_to_i128, u256_to_i128,
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

    /// From a validated multi-feed adapter payload (RedStone/Xoxno wire ABI):
    /// both provider timestamps must not sit in the future relative to the
    /// ledger clock `now_secs`.
    pub(crate) fn from_multi_feed(
        env: &Env,
        now_secs: u64,
        price_data: &RedStonePriceData,
        decimals: u32,
    ) -> Self {
        let package_ts = millis_to_seconds(price_data.package_timestamp);
        let write_ts = millis_to_seconds(price_data.write_timestamp);
        check_not_future_at(env, now_secs, package_ts);
        check_not_future_at(env, now_secs, write_ts);

        let raw_price = u256_to_i128(env, &price_data.price);
        OracleObservation {
            price_wad: normalize_positive_price(env, raw_price, decimals),
            observed_at: write_ts,
            published_at: Some(package_ts),
        }
    }

    /// From a Reflector spot payload; the feed timestamp must not sit in the
    /// future relative to the ledger clock `now_secs`.
    pub(crate) fn from_reflector(
        env: &Env,
        now_secs: u64,
        price_data: &ReflectorPriceData,
        decimals: u32,
    ) -> Self {
        check_not_future_at(env, now_secs, price_data.timestamp);
        OracleObservation {
            price_wad: normalize_positive_price(env, price_data.price, decimals),
            observed_at: price_data.timestamp,
            published_at: None,
        }
    }

    /// Non-panicking [`Self::from_multi_feed`] for the soft status path:
    /// `None` for a future timestamp, non-positive/overflowing price, or a
    /// WAD upscale overflow, instead of reverting the diagnostic view.
    pub(crate) fn try_from_multi_feed(
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
            observed_at: write_ts,
            published_at: Some(package_ts),
        })
    }

    /// Non-panicking [`Self::from_reflector`] for the soft status path.
    pub(crate) fn try_from_reflector(
        now_secs: u64,
        price_data: &ReflectorPriceData,
        decimals: u32,
    ) -> Option<Self> {
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
