//! Oracle price resolution for Reflector and RedStone market sources.
//!
//! Providers return USD WAD prices plus timestamps and decimals. The controller
//! composes primary and optional anchor sources, applies the caller's
//! `OraclePolicy`, and stores the market index used by risk checks.
//!
//! Price flow: `price::token_price` → `compose::resolve_components` (read the
//! `primary` source and, under `PrimaryWithAnchor`, the `anchor`) →
//! `tolerance::calculate_final_price` (band selection) → the unconditional
//! `price::token_price` gates (positive price, sanity bounds, clock-skew). The
//! caller's `OraclePolicy` (`policy.rs`) decides what degradation may be
//! tolerated. The `primary` is the value the protocol prices on; the `anchor`
//! is the independent cross-check.

mod compose;
mod observation;
pub mod policy;
#[cfg(not(feature = "certora"))]
mod price;
#[cfg(feature = "certora")]
#[path = "../../../../verification/certora/controller/harness/oracle_price.rs"]
mod price;
pub(crate) mod providers;
#[cfg(not(feature = "certora"))]
pub(crate) mod tolerance;
#[cfg(feature = "certora")]
#[path = "../../../../verification/certora/controller/harness/oracle_tolerance.rs"]
pub(crate) mod tolerance;
pub(crate) mod validation;

use soroban_sdk::Address;

use crate::cache::Cache;

pub use compose::ResolvedOracleComponents;

// Certora rules reach tolerance helpers through `crate::oracle::*`.
#[cfg(feature = "certora")]
pub(crate) use tolerance::{calculate_final_price, is_within_anchor};

#[cfg(feature = "certora")]
pub(crate) use compose::certora;

pub use price::{token_price, update_asset_index};

pub fn price_components(cache: &mut Cache, asset: &Address) -> ResolvedOracleComponents {
    let market = cache.cached_market_config(asset);
    let configs = market.oracle_config;
    compose::resolve_components(cache, &configs)
}