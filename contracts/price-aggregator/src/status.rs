//! Soft price diagnostics for views: report stale / deviation without reverting.

use common::oracle::observation::is_stale;
use common::types::{AssetOracleConfig, OracleStrategy, PriceStatus};
use soroban_sdk::Address;

use crate::context::ResolutionContext;
use crate::observation::OracleObservation;
use crate::providers;
use crate::tolerance;

/// Soft-resolves one asset into a diagnostic [`PriceStatus`].
///
/// Missing config, unreadable feeds, or hard provider failures yield
/// [`PriceStatus::unusable`] (or partial legs when only one side is readable).
/// Staleness and dual-source deviation set flags instead of panicking.
pub(crate) fn resolve_price_status(cache: &mut ResolutionContext, asset: &Address) -> PriceStatus {
    let Some(config) = cache.cached_asset_oracle_opt(asset) else {
        return PriceStatus::unusable();
    };
    if config.is_pending(asset) {
        return PriceStatus::unusable();
    }

    let primary_max_stale = config
        .primary
        .max_stale_seconds(config.max_price_stale_seconds);
    let Some(primary) = providers::try_read_source(cache, &config.primary) else {
        return PriceStatus::unusable();
    };

    let now = cache.ledger_timestamp_secs();
    let primary_stale = is_stale(now, primary.timestamp(), primary_max_stale);
    let primary_wad = primary.price_wad;

    match config.strategy {
        OracleStrategy::Single => {
            let final_wad = primary_wad;
            let stale = primary_stale;
            let deviation = false;
            let valid = is_valid(final_wad, stale, deviation, &config);
            PriceStatus {
                final_wad,
                primary_wad,
                secondary_wad: final_wad,
                price_timestamp: primary.timestamp(),
                stale,
                deviation,
                valid,
            }
        }
        OracleStrategy::PrimaryWithAnchor => {
            resolve_anchored_status(cache, &config, primary, primary_stale, primary_wad, now)
        }
    }
}

fn resolve_anchored_status(
    cache: &mut ResolutionContext,
    config: &AssetOracleConfig,
    primary: OracleObservation,
    primary_stale: bool,
    primary_wad: i128,
    now: u64,
) -> PriceStatus {
    let Some(anchor_cfg) = config.anchor.as_ref() else {
        // Dual strategy without anchor: primary only, never valid.
        return PriceStatus {
            final_wad: 0,
            primary_wad,
            secondary_wad: 0,
            price_timestamp: primary.timestamp(),
            stale: primary_stale,
            deviation: true,
            valid: false,
        };
    };

    let anchor_max_stale = anchor_cfg.max_stale_seconds(config.max_price_stale_seconds);
    let Some(anchor) = providers::try_read_source(cache, anchor_cfg) else {
        return PriceStatus {
            final_wad: 0,
            primary_wad,
            secondary_wad: 0,
            price_timestamp: primary.timestamp(),
            stale: primary_stale,
            deviation: true,
            valid: false,
        };
    };

    let anchor_stale = is_stale(now, anchor.timestamp(), anchor_max_stale);
    let stale = primary_stale || anchor_stale;
    let secondary_wad = anchor.price_wad;
    let price_timestamp = primary.timestamp().min(anchor.timestamp());

    let within_band = tolerance::within_tolerance_band(
        cache.env(),
        secondary_wad,
        primary_wad,
        &config.tolerance,
    );
    let deviation = !within_band;
    // Still surface a midpoint for UI when both legs exist; validity requires band.
    let final_wad = tolerance::midpoint_price_or_zero(cache.env(), secondary_wad, primary_wad);
    let valid = is_valid(final_wad, stale, deviation, config);

    PriceStatus {
        final_wad,
        primary_wad,
        secondary_wad,
        price_timestamp,
        stale,
        deviation,
        valid,
    }
}

fn is_valid(final_wad: i128, stale: bool, deviation: bool, config: &AssetOracleConfig) -> bool {
    if stale || deviation || final_wad <= 0 {
        return false;
    }
    if config.max_sanity_price_wad <= 0 {
        return false;
    }
    final_wad >= config.min_sanity_price_wad && final_wad <= config.max_sanity_price_wad
}
