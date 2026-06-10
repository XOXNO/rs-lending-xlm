//! Oracle price resolution for Reflector and RedStone sources.
//!
//! Reads produce USD WAD prices, timestamps, and decimals. The primary source
//! prices the market; an anchor is an independent tolerance check.

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
