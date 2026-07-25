//! Hard-path USD price resolution (`resolve_usd_price`). Fail-closed.

use common::errors::{GenericError, OracleError};
use common::types::{AssetOracleConfig, OracleSourceConfig, PriceFeedRaw};
use soroban_sdk::{assert_with_error, panic_with_error, Address};

use crate::compose::{self, Composition, Leg, SourceKind};
use crate::context::ResolutionContext;
use crate::observation::OracleObservation;
use crate::providers;
use crate::tolerance;

/// Hard-path resolution result; per-leg diagnostics live in `status`.
pub(crate) struct ResolvedPrice {
    pub final_price_wad: i128,
    pub timestamp: u64,
}

/// Cached USD price; miss resolves under cycle guard.
pub(crate) fn resolve_usd_price(cache: &mut ResolutionContext, asset: &Address) -> PriceFeedRaw {
    if let Some(feed) = cache.cached_price(asset) {
        return feed;
    }

    // Cycle guard: re-entry of an in-flight asset reverts `OracleCycleDetected`.
    cache.push_resolution(asset);

    // Missing `AssetOracle` → `OracleNotConfigured` (pending/disabled gate).
    let config = cache.cached_asset_oracle(asset);
    let feed = resolve_with_config(cache, asset, &config);
    cache.store_price(asset, feed.clone());
    cache.pop_resolution();
    feed
}

/// Resolves a USD price without writing a cache entry.
pub(crate) fn resolve_with_config(
    cache: &mut ResolutionContext,
    asset: &Address,
    config: &AssetOracleConfig,
) -> PriceFeedRaw {
    let resolved = resolve_guarded(cache, asset, config);
    PriceFeedRaw {
        price_wad: resolved.final_price_wad,
        asset_decimals: config.asset_decimals,
        timestamp: resolved.timestamp,
    }
}

/// Resolves oracle components and applies every fail-closed check: pending
/// sentinel, positive final price, and the configured sanity band.
pub(crate) fn resolve_guarded(
    cache: &mut ResolutionContext,
    asset: &Address,
    config: &AssetOracleConfig,
) -> ResolvedPrice {
    // Reject the `AssetOracleConfig::pending_for` self-pointer sentinel.
    assert_with_error!(
        cache.env(),
        !config.is_pending(asset),
        OracleError::OracleNotConfigured
    );
    let composition = compose::compose(cache, config);
    let resolved = render_composition(cache, config, &composition);
    assert_with_error!(
        cache.env(),
        resolved.final_price_wad > 0,
        OracleError::InvalidPrice
    );
    if config.max_sanity_price_wad <= 0
        || resolved.final_price_wad < config.min_sanity_price_wad
        || resolved.final_price_wad > config.max_sanity_price_wad
    {
        panic_with_error!(cache.env(), OracleError::SanityBoundViolated);
    }
    resolved
}

/// Renders a [`Composition`] fail-closed: every leg must be readable and fresh,
/// a dual strategy must carry an anchor, and a dual pair must agree inside the
/// tolerance band. Walked in the order the error contract fixes — primary
/// readable, primary fresh, anchor present, anchor readable, anchor fresh,
/// band — so a config broken in several ways reverts with the first error.
fn render_composition(
    cache: &mut ResolutionContext,
    config: &AssetOracleConfig,
    composition: &Composition,
) -> ResolvedPrice {
    let primary = require_leg(cache, &config.primary, &composition.primary);

    match (config.anchor.as_ref(), composition.anchor.as_ref()) {
        // Missing anchor on dual strategy fails closed with NoLastPrice (#210),
        // matching the read-time backstop.
        (_, None) if composition.dual_missing_anchor => {
            panic_with_error!(cache.env(), OracleError::NoLastPrice)
        }
        (Some(anchor_source), Some(anchor_leg)) => {
            let anchor = require_leg(cache, anchor_source, anchor_leg);
            let final_price_wad = tolerance::midpoint_if_in_band(
                cache.env(),
                anchor.price_wad,
                primary.price_wad,
                &config.tolerance,
            );
            ResolvedPrice {
                final_price_wad,
                // Blend freshness is the older leg.
                timestamp: core::cmp::min(primary.timestamp(), anchor.timestamp()),
            }
        }
        // Single strategy: a configured-but-unused anchor source is ignored,
        // exactly as `compose` ignores it.
        _ => ResolvedPrice {
            final_price_wad: primary.price_wad,
            timestamp: primary.timestamp(),
        },
    }
}

/// Requires a leg to be readable and fresh, in that order.
fn require_leg<'a>(
    cache: &mut ResolutionContext,
    source: &OracleSourceConfig,
    leg: &'a Leg,
) -> &'a OracleObservation {
    match leg.result.as_ref() {
        Err(kind) => reject_leg(cache, source, *kind),
        Ok(observation) => {
            if leg.stale {
                panic_with_error!(cache.env(), OracleError::PriceFeedStale);
            }
            observation
        }
    }
}

/// Raises the error a failed leg used to raise from inside the provider layer.
///
/// `compose` reads softly, which reports a missing feed and a present-but-
/// rejected payload (non-positive price, future timestamp, `i128` overflow)
/// alike as one unreadable leg. The required read accepts exactly what the soft
/// read accepts, so replaying it here reverts with the provider's own code
/// instead of one guessed from the source family. The family error remains the
/// backstop for the missing-feed case.
fn reject_leg(cache: &mut ResolutionContext, source: &OracleSourceConfig, kind: SourceKind) -> ! {
    providers::read_required_source(cache, source);
    match kind {
        SourceKind::Reflector => panic_with_error!(cache.env(), OracleError::NoLastPrice),
        SourceKind::MultiFeed => panic_with_error!(cache.env(), GenericError::InvalidTicker),
    }
}

#[cfg(test)]
#[path = "../tests/oracle/hard_path_errors.rs"]
mod hard_path_error_tests;
