//! Composes required oracle sources into a final price.

use common::errors::OracleError;
use common::oracle::observation::is_stale;
use common::types::{AssetOracleConfig, OracleStrategy};
use soroban_sdk::panic_with_error;

use crate::context::ResolutionContext;
use crate::observation::OracleObservation;
use crate::providers;
use crate::tolerance::midpoint_if_in_band;

pub(crate) struct ResolvedPrice {
    pub primary_price_wad: i128,
    pub anchor_price_wad: Option<i128>,
    pub final_price_wad: i128,
    pub timestamp: u64,
}

impl ResolvedPrice {
    /// `(primary_wad, secondary_wad)` legs for the views ABI.
    ///
    /// Secondary is the anchor when present; otherwise it equals final (and
    /// primary) under PrimaryOnly (`OracleStrategy::Single`).
    pub fn primary_and_secondary(&self) -> (i128, i128) {
        let secondary_wad = self.anchor_price_wad.unwrap_or(self.final_price_wad);
        (self.primary_price_wad, secondary_wad)
    }
}

pub(crate) fn resolve_components(
    cache: &mut ResolutionContext,
    config: &AssetOracleConfig,
) -> ResolvedPrice {
    let primary_max_stale = config
        .primary
        .max_stale_seconds(config.max_price_stale_seconds);
    let primary = providers::read_required_source(cache, &config.primary, primary_max_stale);
    require_fresh(cache, &primary, primary_max_stale);

    match config.strategy {
        OracleStrategy::Single => ResolvedPrice {
            primary_price_wad: primary.price_wad,
            anchor_price_wad: None,
            final_price_wad: primary.price_wad,
            timestamp: primary.timestamp(),
        },
        OracleStrategy::PrimaryWithAnchor => {
            // Missing anchor on dual strategy fails closed with NoLastPrice (#210),
            // matching the historical read-time backstop.
            let anchor_config = config
                .anchor
                .as_ref()
                .unwrap_or_else(|| panic_with_error!(cache.env(), OracleError::NoLastPrice));
            let anchor_max_stale = anchor_config.max_stale_seconds(config.max_price_stale_seconds);
            let anchor = providers::read_required_source(cache, anchor_config, anchor_max_stale);
            require_fresh(cache, &anchor, anchor_max_stale);

            let final_price_wad = midpoint_if_in_band(
                cache.env(),
                anchor.price_wad,
                primary.price_wad,
                &config.tolerance,
            );
            // Blend freshness is the older leg.
            let timestamp = core::cmp::min(primary.timestamp(), anchor.timestamp());

            ResolvedPrice {
                primary_price_wad: primary.price_wad,
                anchor_price_wad: Some(anchor.price_wad),
                final_price_wad,
                timestamp,
            }
        }
    }
}

/// Reverts `PriceFeedStale` when the observation exceeds `max_stale`.
fn require_fresh(cache: &ResolutionContext, observation: &OracleObservation, max_stale: u64) {
    if is_stale(
        cache.ledger_timestamp_secs(),
        observation.timestamp(),
        max_stale,
    ) {
        panic_with_error!(cache.env(), OracleError::PriceFeedStale);
    }
}

#[cfg(test)]
#[path = "../tests/oracle/compose.rs"]
mod tests;
