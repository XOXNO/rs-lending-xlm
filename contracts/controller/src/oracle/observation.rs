//! Oracle observation construction.
//!
//! Converts raw Reflector/RedStone responses into `OracleObservation`.
//! Normalization, staleness, and clock-skew guards live in
//! `common::oracle::observation`.

use common::oracle::observation::normalize_positive_price;
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
    /// Freshness timestamp: minimum of `published_at` and `observed_at` when both
    /// are set. Reflector quoted-base repricing may further tighten
    /// `observed_at` against the USD quote feed in `reprice_to_usd`.
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
