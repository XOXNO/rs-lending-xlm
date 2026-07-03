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

pub use compose::ResolvedOracleComponents;

// Certora rules reach tolerance helpers through `crate::oracle::*`.
#[cfg(feature = "certora")]
pub(crate) use tolerance::calculate_final_price;

pub(crate) use prefetch::prefetch_redstone_feeds;
pub use price::{price_with_config, token_price};

pub fn price_components(cache: &mut Cache, hub_asset: &HubAssetKey) -> ResolvedOracleComponents {
    // Pricing is token-rooted (hub-independent), keyed by the bare asset:
    // `resolve_oracle_config` panics `OracleNotConfigured` for any asset with no
    // `AssetOracle` entry (unlisted, pending, or disabled).
    let configs = cache.resolve_oracle_config(&hub_asset.asset);
    compose::resolve_components(cache, &configs)
}
