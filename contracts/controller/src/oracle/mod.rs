//! Oracle price resolution for Reflector and RedStone market sources.
//!
//! Providers return USD WAD prices plus timestamps and decimals. The controller
//! composes primary and optional anchor sources, applies the caller's
//! `OraclePolicy`, and stores the market index used by risk checks.

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

use crate::cache::ControllerCache;

// Certora rules reach tolerance helpers through `crate::oracle::*`.
#[cfg(feature = "certora")]
pub(crate) use tolerance::{
    bps_i128_to_u32, calculate_final_price, calculate_tolerance_range, is_within_anchor,
    validate_and_calculate_tolerances,
};

pub use price::{token_price, update_asset_index};

pub struct PriceComponents {
    pub aggregator_price_wad: Option<i128>,
    pub safe_price_wad: Option<i128>,
    pub final_price_wad: i128,
    pub within_first_tolerance: bool,
    pub within_second_tolerance: bool,
}

pub fn price_components(cache: &mut ControllerCache, asset: &Address) -> PriceComponents {
    let market = cache.cached_market_config(asset);
    let configs = market.oracle_config;
    let components = compose::resolve_components(cache, &configs);
    PriceComponents {
        aggregator_price_wad: components.anchor_price_wad,
        safe_price_wad: components.primary_price_wad,
        final_price_wad: components.final_price_wad,
        within_first_tolerance: components.within_first_tolerance,
        within_second_tolerance: components.within_second_tolerance,
    }
}
