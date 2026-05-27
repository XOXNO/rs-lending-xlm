//! Price composition: primary vs anchor, strategy dispatch (SpotVsTwap /
//! DualOracle), tolerance application, and final price selection.
//!
//! This is the narrow waist that turns two (or one) `OracleObservation`s
//! into the single `PriceFeedRaw` that the rest of the controller sees.
//! It is deliberately thin; all provider-specific logic lives in the
//! provider modules, all tolerance math lives in `tolerance`.

use common::errors::OracleError;
use common::types::{MarketOracleConfig, OracleStrategy};
use soroban_sdk::{assert_with_error, panic_with_error};

use super::observation::{is_stale, OracleObservation};
use super::providers;
use super::tolerance::{calculate_final_price, is_within_anchor};
use crate::cache::ControllerCache;

#[cfg_attr(feature = "certora", allow(dead_code))] // Struct only used via resolve_price in non-certora price.rs; harness replaces that module.
pub(crate) struct ResolvedOraclePrice {
    pub price_wad: i128,
    pub timestamp: u64,
}

pub(crate) struct ResolvedOracleComponents {
    pub primary_price_wad: Option<i128>,
    pub anchor_price_wad: Option<i128>,
    pub final_price_wad: i128,
    pub timestamp: u64,
    pub within_first_tolerance: bool,
    pub within_second_tolerance: bool,
}

#[cfg_attr(feature = "certora", allow(dead_code))] // Fn only called from non-certora price.rs (harness replaces price resolution).
pub(crate) fn resolve_price(
    cache: &mut ControllerCache,
    config: &MarketOracleConfig,
) -> ResolvedOraclePrice {
    let components = resolve_components(cache, config);
    ResolvedOraclePrice {
        price_wad: components.final_price_wad,
        timestamp: components.timestamp,
    }
}

pub(crate) fn resolve_components(
    cache: &mut ControllerCache,
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
            // The second band is the wider `last` tolerance. `validate_oracle_bounds`
            // enforces last > first, so the first band is a strict subset of the
            // last; within_first therefore implies within_second.
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

fn validate_primary_freshness(
    cache: &ControllerCache,
    observation: &OracleObservation,
    max_stale: u64,
) {
    if is_stale(
        cache.current_timestamp_ms / 1000,
        observation.timestamp(),
        max_stale,
    ) && !cache.oracle_policy.allows_stale_source()
    {
        panic_with_error!(cache.env(), OracleError::PriceFeedStale);
    }
}

fn anchor_is_usable(
    cache: &ControllerCache,
    observation: &OracleObservation,
    max_stale: u64,
) -> bool {
    if is_stale(
        cache.current_timestamp_ms / 1000,
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

fn fallback_to_primary(
    cache: &ControllerCache,
    primary: OracleObservation,
) -> ResolvedOracleComponents {
    assert_with_error!(
        cache.env(),
        cache.oracle_policy.allows_missing_twap_fallback(),
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
