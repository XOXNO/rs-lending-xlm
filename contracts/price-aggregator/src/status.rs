//! Soft price diagnostics for views: report stale / deviation without reverting.
//!
//! Every rule that decides an outcome — which sources the strategy calls for,
//! whether a leg is past its max-stale window, whether a dual pair agrees inside
//! the tolerance band — is applied once by `compose` and only rendered here, so
//! this view and the fail-closed `price` path cannot drift apart. What remains
//! here is presentation: which [`PriceStatus`] shape each outcome takes, and
//! `is_valid`, the soft mirror of the hard path's positive-price and sanity-band
//! gates.

use common::types::{AssetOracleConfig, PriceStatus};
use soroban_sdk::Address;

use crate::compose::{self, Composition};
use crate::context::ResolutionContext;
use crate::observation::OracleObservation;
use crate::tolerance;

/// Soft-resolves one asset into a diagnostic [`PriceStatus`], memoized for the
/// rest of the transaction.
///
/// Missing config, unreadable feeds, or hard provider failures yield
/// [`PriceStatus::unusable`] (or partial legs when only one side is readable).
/// Staleness and dual-source deviation set flags instead of panicking.
///
/// The memo matters beyond the bulk view: a Reflector leg with a quoted base
/// reprices through this, so assets sharing a quote resolve it once per
/// transaction instead of once each.
pub(crate) fn resolve_price_status(cache: &mut ResolutionContext, asset: &Address) -> PriceStatus {
    if let Some(status) = cache.cached_status(asset) {
        return status;
    }
    let status = compute_price_status(cache, asset);
    cache.store_status(asset, status.clone());
    status
}

fn compute_price_status(cache: &mut ResolutionContext, asset: &Address) -> PriceStatus {
    let Some(config) = cache.cached_asset_oracle_opt(asset) else {
        return PriceStatus::unusable();
    };
    if config.is_pending(asset) {
        return PriceStatus::unusable();
    }

    // The gate stops at the first unreadable leg, because the renderer below
    // returns `unusable()` on an unreadable primary without ever consulting the
    // anchor. Reading it anyway would not just waste a cross-contract call: a
    // soft read is not panic-free, so an anchor whose Reflector contract
    // reverts at read time — paused, archived, or upgraded, with nothing wrong
    // in the config — would revert this view and, through a quoted base, the
    // fail-closed path that reprices on it.
    let composition = compose::compose(cache, &config, |_, _, leg| leg.result.is_ok());
    let Ok(primary) = composition.primary.result.as_ref() else {
        return PriceStatus::unusable();
    };

    match composition.anchor.as_ref() {
        // No anchor leg and none was called for: a `Single` strategy prices off
        // the primary alone, so there is no second opinion to deviate from.
        None if !composition.dual_missing_anchor => {
            let primary_wad = primary.price_wad;
            let stale = composition.primary.stale;
            PriceStatus {
                final_wad: primary_wad,
                primary_wad,
                secondary_wad: primary_wad,
                price_timestamp: primary.timestamp(),
                stale,
                deviation: false,
                valid: is_valid(primary_wad, stale, false, &config),
            }
        }
        _ => anchored_status(cache, &config, &composition, primary),
    }
}

/// Renders the dual-source cases: an anchor the config omits, an anchor that
/// could not be read, and a readable pair that either agrees or does not.
fn anchored_status(
    cache: &ResolutionContext,
    config: &AssetOracleConfig,
    composition: &Composition,
    primary: &OracleObservation,
) -> PriceStatus {
    // Nothing to price against, because the config carries no anchor source or
    // the one it carries is unreadable. The pair never agreed, which is what
    // `deviation` records, and the view reports only the leg it did read.
    let unpaired = PriceStatus {
        final_wad: 0,
        primary_wad: primary.price_wad,
        secondary_wad: 0,
        price_timestamp: primary.timestamp(),
        stale: composition.primary.stale,
        deviation: true,
        valid: false,
    };
    let Some(anchor_leg) = composition.anchor.as_ref() else {
        return unpaired;
    };
    let Ok(anchor) = anchor_leg.result.as_ref() else {
        return unpaired;
    };

    // Both legs are readable, so `blended` withholds a price for exactly one
    // reason left: the pair fell outside the tolerance band. Its absence is the
    // deviation flag. The view still surfaces the midpoint `blended` withheld —
    // a diagnostic shows the number it rejected — and `deviation` is what keeps
    // that number out of `valid`.
    let (final_wad, price_timestamp, deviation) =
        match composition.blended(cache.env(), &config.tolerance) {
            Some((price_wad, timestamp)) => (price_wad, timestamp, false),
            None => (
                tolerance::midpoint_price_or_zero(cache.env(), anchor.price_wad, primary.price_wad),
                // Blend freshness is the older leg, band agreement or not.
                primary.timestamp().min(anchor.timestamp()),
                true,
            ),
        };
    let stale = composition.primary.stale || anchor_leg.stale;

    PriceStatus {
        final_wad,
        primary_wad: primary.price_wad,
        secondary_wad: anchor.price_wad,
        price_timestamp,
        stale,
        deviation,
        valid: is_valid(final_wad, stale, deviation, config),
    }
}

/// True when a composed price would also survive the fail-closed path: fresh,
/// in band, positive, and inside an enabled sanity band.
///
/// `providers::reflector` leans on this — a quoted base reprices only through a
/// `valid` status — so loosening it loosens `price` itself.
fn is_valid(final_wad: i128, stale: bool, deviation: bool, config: &AssetOracleConfig) -> bool {
    if stale || deviation || final_wad <= 0 {
        return false;
    }
    if config.max_sanity_price_wad <= 0 {
        return false;
    }
    final_wad >= config.min_sanity_price_wad && final_wad <= config.max_sanity_price_wad
}

#[cfg(test)]
#[path = "../tests/oracle/status.rs"]
mod tests;
