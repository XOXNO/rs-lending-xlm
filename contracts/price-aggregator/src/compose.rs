//! Hard-path composition: required sources into a final USD price.
//! Reverts on missing, stale, or out-of-band legs; soft diagnostics live in
//! `status`.

use common::errors::OracleError;
use common::oracle::observation::is_stale;
use common::types::{AssetOracleConfig, OracleStrategy};
use soroban_sdk::panic_with_error;

use crate::context::ResolutionContext;
use crate::observation::OracleObservation;
use crate::providers;
use crate::tolerance::midpoint_if_in_band;

/// Hard-path resolution result; per-leg diagnostics live in `status`.
pub(crate) struct ResolvedPrice {
    pub final_price_wad: i128,
    pub timestamp: u64,
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
            final_price_wad: primary.price_wad,
            timestamp: primary.timestamp(),
        },
        OracleStrategy::PrimaryWithAnchor => {
            // Missing anchor on dual strategy fails closed with NoLastPrice (#210),
            // matching the read-time backstop.
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
