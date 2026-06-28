//! # Oracle — USD price resolution for Reflector and RedStone sources.
//!
//! One strict, fail-closed path: every source is required and must be fresh,
//! and dual-source markets blend within the tolerance band or revert. Views
//! resolve prices the same way, so a view reverts exactly when a transaction
//! would.
//!
//! Call trace: `token_price` (status/sanity gates + price cache) →
//! `compose::resolve_components`, which reads the primary and, in dual-source
//! markets, the anchor via `providers::read_required_source` — each normalized
//! via `observation` — then blends the pair through
//! `tolerance::calculate_final_price`. A quoted-base Reflector source reprices
//! by recursing through `token_price` for the quote asset
//! (`providers::reflector::resolve_usd_quote`). `price_components` exposes the
//! same resolution to views.

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

use controller_interface::types::HubAssetKey;

use crate::cache::Cache;

pub use compose::ResolvedOracleComponents;

// Certora rules reach tolerance helpers through `crate::oracle::*`.
#[cfg(feature = "certora")]
pub(crate) use tolerance::calculate_final_price;

pub(crate) use prefetch::prefetch_redstone_feeds;
pub use price::token_price;

pub fn price_components(cache: &mut Cache, hub_asset: &HubAssetKey) -> ResolvedOracleComponents {
    // Reject unlisted `(hub, asset)` with `AssetNotSupported`; `resolve_oracle_config`
    // then panics `OracleNotConfigured` for a listed-but-pending/disabled asset.
    // The listed-gate uses the real hub from the key; the price itself is
    // token-rooted (hub-independent) and keyed by the bare asset.
    let env = cache.env().clone();
    crate::validation::require_asset_supported(&env, cache, hub_asset);
    let configs = cache.resolve_oracle_config(&hub_asset.asset);
    compose::resolve_components(cache, &configs)
}
