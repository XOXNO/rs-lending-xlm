//! Provider observations normalized to WAD with timestamp guards.

use common::oracle::observation::{
    check_not_future_at, millis_to_seconds, normalize_positive_price, u256_to_i128,
};
use common::oracle::providers::redstone::RedStonePriceData;
use common::oracle::providers::reflector::ReflectorPriceData;
use soroban_sdk::Env;

/// Provider price observation consumed by the compose layer.
#[cfg_attr(feature = "certora", allow(dead_code))] // Dead when certora stubs price paths.
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
}

/// Constructor after provider validation: final WAD normalization and assembly.
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

/// Rejects future timestamps and normalizes RedStone price.
pub(crate) fn redstone_observation_from_price_data(
    env: &Env,
    price_data: &RedStonePriceData,
    decimals: u32,
) -> OracleObservation {
    let package_ts = millis_to_seconds(price_data.package_timestamp);
    let write_ts = millis_to_seconds(price_data.write_timestamp);
    let now_secs = env.ledger().timestamp();
    check_not_future_at(env, now_secs, package_ts);
    check_not_future_at(env, now_secs, write_ts);

    let raw_price = u256_to_i128(env, &price_data.price);
    build_observation(env, raw_price, decimals, write_ts, Some(package_ts))
}

/// Rejects future timestamps and normalizes Reflector price.
pub(crate) fn reflector_observation_from_price_data(
    env: &Env,
    pd: &ReflectorPriceData,
    decimals: u32,
) -> OracleObservation {
    check_not_future_at(env, env.ledger().timestamp(), pd.timestamp);
    build_observation(env, pd.price, decimals, pd.timestamp, None)
}
