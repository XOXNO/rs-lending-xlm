//! Composes required oracle sources into a final price.

use common::errors::OracleError;
use common::oracle::observation::is_stale;
use common::types::{MarketOracleConfig, OracleStrategy};
use soroban_sdk::panic_with_error;

use crate::context::Cache;
use crate::oracle::observation::OracleObservation;
use crate::oracle::providers;
use crate::oracle::tolerance::calculate_final_price;

pub(crate) struct ResolvedOracleComponents {
    pub primary_price_wad: Option<i128>,
    pub anchor_price_wad: Option<i128>,
    pub final_price_wad: i128,
    #[allow(dead_code)]
    pub timestamp: u64,
}

impl ResolvedOracleComponents {
    /// Returns the (safe, aggregator) ABI price pair, defaulting each leg to the final price.
    pub fn to_abi_prices(&self) -> (i128, i128) {
        let safe_price_wad = self.primary_price_wad.unwrap_or(self.final_price_wad);
        let aggregator_price_wad = self.anchor_price_wad.unwrap_or(self.final_price_wad);
        (safe_price_wad, aggregator_price_wad)
    }
}

/// Reads and freshness-checks the required source(s) and blends them into the final price.
pub(crate) fn resolve_components(
    cache: &mut Cache,
    config: &MarketOracleConfig,
) -> ResolvedOracleComponents {
    let primary_max_stale = config
        .primary
        .max_stale_seconds(config.max_price_stale_seconds);
    let primary = providers::read_required_source(cache, &config.primary, primary_max_stale);
    require_fresh(cache, &primary, primary_max_stale);

    match config.strategy {
        OracleStrategy::Single => ResolvedOracleComponents {
            primary_price_wad: Some(primary.price_wad),
            anchor_price_wad: None,
            final_price_wad: primary.price_wad,
            timestamp: primary.timestamp(),
        },
        OracleStrategy::PrimaryWithAnchor => {
            let anchor_config = config
                .anchor
                .as_ref()
                .unwrap_or_else(|| panic_with_error!(cache.env(), OracleError::NoLastPrice));
            let anchor_max_stale = anchor_config.max_stale_seconds(config.max_price_stale_seconds);
            let anchor = providers::read_required_source(cache, anchor_config, anchor_max_stale);
            require_fresh(cache, &anchor, anchor_max_stale);

            let final_price_wad = calculate_final_price(
                cache.env(),
                anchor.price_wad,
                primary.price_wad,
                &config.tolerance,
            );
            // The price is always a blend, so it is only as fresh as the older leg.
            let timestamp = core::cmp::min(primary.timestamp(), anchor.timestamp());

            ResolvedOracleComponents {
                primary_price_wad: Some(primary.price_wad),
                anchor_price_wad: Some(anchor.price_wad),
                final_price_wad,
                timestamp,
            }
        }
    }
}

/// Reverts `PriceFeedStale` when the observation exceeds `max_stale`.
fn require_fresh(cache: &Cache, observation: &OracleObservation, max_stale: u64) {
    if is_stale(
        cache.ledger_timestamp_secs(),
        observation.timestamp(),
        max_stale,
    ) {
        panic_with_error!(cache.env(), OracleError::PriceFeedStale);
    }
}

#[cfg(test)]
#[path = "../../tests/oracle/compose.rs"]
mod tests;
