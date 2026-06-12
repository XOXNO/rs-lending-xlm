//! Composes primary and anchor observations into a final price.

use common::errors::OracleError;
use common::oracle::observation::is_stale;
use controller_interface::types::{MarketOracleConfig, OracleStrategy};
use soroban_sdk::{assert_with_error, panic_with_error};

use super::observation::OracleObservation;
use super::providers;
use super::tolerance::{calculate_final_price, is_within_anchor};
use crate::cache::Cache;

pub struct ResolvedOracleComponents {
    pub primary_price_wad: Option<i128>,
    pub anchor_price_wad: Option<i128>,
    pub final_price_wad: i128,
    pub timestamp: u64,
    pub within_first_tolerance: bool,
    pub within_second_tolerance: bool,
}

impl ResolvedOracleComponents {
    /// Maps primary and anchor prices to ABI fields.
    pub fn to_abi_prices(&self) -> (i128, i128) {
        let safe_price_wad = self.primary_price_wad.unwrap_or(self.final_price_wad);
        let aggregator_price_wad = self.anchor_price_wad.unwrap_or(self.final_price_wad);
        (safe_price_wad, aggregator_price_wad)
    }
}

pub(crate) fn resolve_components(
    cache: &mut Cache,
    config: &MarketOracleConfig,
) -> ResolvedOracleComponents {
    let primary_max_stale = config
        .primary
        .max_stale_seconds(config.max_price_stale_seconds);
    let primary = providers::read_source(cache, &config.primary, primary_max_stale, true)
        .unwrap_or_else(|| panic_with_error!(cache.env(), OracleError::NoLastPrice));
    validate_primary_freshness(cache, &primary, primary_max_stale);

    match config.strategy {
        OracleStrategy::Single => ResolvedOracleComponents {
            primary_price_wad: Some(primary.price_wad),
            anchor_price_wad: None,
            final_price_wad: primary.price_wad,
            timestamp: primary.timestamp(),
            within_first_tolerance: true,
            within_second_tolerance: true,
        },
        OracleStrategy::PrimaryWithAnchor => {
            let Some(anchor_config) = config.anchor.as_ref() else {
                return fallback_to_primary(cache, primary);
            };
            let anchor_max_stale = anchor_config.max_stale_seconds(config.max_price_stale_seconds);
            let Some(anchor) =
                providers::read_source(cache, anchor_config, anchor_max_stale, false)
            else {
                return fallback_to_primary(cache, primary);
            };

            if !anchor_is_usable(cache, &anchor, anchor_max_stale) {
                return fallback_to_primary(cache, primary);
            }

            let final_price = calculate_final_price(
                cache,
                Some(anchor.price_wad),
                Some(primary.price_wad),
                &config.tolerance,
            );
            let within_first = is_within_anchor(
                cache.env(),
                anchor.price_wad,
                primary.price_wad,
                config.tolerance.first_upper_ratio_bps,
                config.tolerance.first_lower_ratio_bps,
            );
            // The second band is the wider `last` tolerance. `require_last_tolerance_gt_first`
            // enforces last > first at configure time, so the first band is a strict
            // subset of the last; within_first therefore implies within_second.
            let within_second = is_within_anchor(
                cache.env(),
                anchor.price_wad,
                primary.price_wad,
                config.tolerance.last_upper_ratio_bps,
                config.tolerance.last_lower_ratio_bps,
            );
            let timestamp = if final_price == primary.price_wad {
                primary.timestamp()
            } else {
                core::cmp::min(primary.timestamp(), anchor.timestamp())
            };

            ResolvedOracleComponents {
                primary_price_wad: Some(primary.price_wad),
                anchor_price_wad: Some(anchor.price_wad),
                final_price_wad: final_price,
                timestamp,
                within_first_tolerance: within_first,
                within_second_tolerance: within_second,
            }
        }
    }
}

fn validate_primary_freshness(cache: &Cache, observation: &OracleObservation, max_stale: u64) {
    if is_stale(
        cache.ledger_timestamp_secs(),
        observation.timestamp(),
        max_stale,
    ) && !cache.oracle_policy.allows_stale_source()
    {
        panic_with_error!(cache.env(), OracleError::PriceFeedStale);
    }
}

fn anchor_is_usable(cache: &Cache, observation: &OracleObservation, max_stale: u64) -> bool {
    if is_stale(
        cache.ledger_timestamp_secs(),
        observation.timestamp(),
        max_stale,
    ) {
        if cache.oracle_policy.allows_stale_source() {
            return false;
        }
        panic_with_error!(cache.env(), OracleError::PriceFeedStale);
    }
    true
}

fn fallback_to_primary(cache: &Cache, primary: OracleObservation) -> ResolvedOracleComponents {
    assert_with_error!(
        cache.env(),
        cache.oracle_policy.allows_degraded_dual_source(),
        OracleError::NoLastPrice
    );
    ResolvedOracleComponents {
        primary_price_wad: Some(primary.price_wad),
        anchor_price_wad: None,
        final_price_wad: primary.price_wad,
        timestamp: primary.timestamp(),
        within_first_tolerance: false,
        within_second_tolerance: false,
    }
}

/// Certora-only hooks for compose policy and degradation paths.
#[cfg(feature = "certora")]
pub(crate) mod certora {
    use super::*;

    pub fn observation(price_wad: i128, timestamp: u64) -> OracleObservation {
        OracleObservation {
            price_wad,
            observed_at: timestamp,
            published_at: None,
        }
    }

    pub fn fallback_to_primary(
        cache: &Cache,
        primary_price_wad: i128,
        timestamp: u64,
    ) -> ResolvedOracleComponents {
        super::fallback_to_primary(cache, observation(primary_price_wad, timestamp))
    }

    pub fn anchor_is_usable(
        cache: &Cache,
        observation: &OracleObservation,
        max_stale: u64,
    ) -> bool {
        super::anchor_is_usable(cache, observation, max_stale)
    }
}
