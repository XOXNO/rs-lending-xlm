//! # Oracle — USD price resolution for Reflector and RedStone sources.
//!
//! Each entrypoint sets a [`policy::OraclePolicy`] on the `Cache` that decides which
//! degradations the flow tolerates; risk-increasing flows fail closed.
//!
//! Call trace: `token_price` (status/sanity gates + price cache) →
//! `compose::resolve_components`, which calls `providers::read_source` for the primary
//! (required) and, in dual-source markets, the anchor (optional) — each normalized via
//! `observation` — then resolves the pair through `tolerance::calculate_final_price`. A
//! quoted-base Reflector source reprices by recursing through `token_price` for the quote
//! asset (`providers::reflector::resolve_usd_quote`). `price_components` exposes the same
//! resolution to views without the gates.

mod compose;
mod observation;
pub mod policy;
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

use soroban_sdk::Address;

use crate::cache::Cache;

pub use compose::ResolvedOracleComponents;

// Certora rules reach tolerance helpers through `crate::oracle::*`.
#[cfg(feature = "certora")]
pub(crate) use tolerance::{calculate_final_price, is_within_anchor};

#[cfg(feature = "certora")]
pub(crate) use compose::certora;

pub(crate) use prefetch::prefetch_redstone_feeds;
pub use price::token_price;

pub fn price_components(cache: &mut Cache, asset: &Address) -> ResolvedOracleComponents {
    let market = cache.cached_market_config(asset);
    let configs = market.oracle_config;
    compose::resolve_components(cache, &configs)
}
