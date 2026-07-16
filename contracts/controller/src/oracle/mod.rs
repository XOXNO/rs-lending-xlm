//! USD oracle resolution with fail-closed sources and tolerance checks.

mod compose;
mod observation;
mod prefetch;
#[cfg(not(feature = "certora"))]
mod price;
#[cfg(feature = "certora")]
#[path = "../../../../certora/controller/harness/oracle_price.rs"]
mod price;
pub(crate) mod providers;
#[cfg(not(feature = "certora"))]
pub(crate) mod tolerance;
#[cfg(feature = "certora")]
#[path = "../../../../certora/controller/harness/oracle_tolerance.rs"]
pub(crate) mod tolerance;

use common::types::HubAssetKey;

use crate::context::Cache;

pub(crate) use compose::ResolvedOracleComponents;

#[cfg(feature = "certora")]
pub(crate) use tolerance::calculate_final_price;

pub(crate) use prefetch::prefetch_redstone_feeds;
pub(crate) use price::{price_with_config, token_price};

/// Resolves the token-rooted oracle components (primary, anchor, final price) for a hub-asset.
pub(crate) fn price_components(cache: &mut Cache, hub_asset: &HubAssetKey) -> ResolvedOracleComponents {
    // Pricing is token-rooted (hub-independent), keyed by the bare asset:
    // `cached_asset_oracle` panics `OracleNotConfigured` for any asset with no
    // `AssetOracle` entry (unlisted, pending, or disabled).
    let configs = cache.cached_asset_oracle(&hub_asset.asset);
    compose::resolve_components(cache, &configs)
}
