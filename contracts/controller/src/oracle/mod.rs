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
//! is the independent cross-check. (The public ABI view fields name these
//! `safe_price_wad`/`aggregator_price_wad` — see `PriceComponents`.)

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

// Certora rules reach tolerance helpers through `crate::oracle::*`.
#[cfg(feature = "certora")]
pub(crate) use tolerance::{calculate_final_price, is_within_anchor};

pub use price::{token_price, update_asset_index};

/// Per-source breakdown of a resolved price, exposed for views. Field names
/// mirror the public ABI: `aggregator_price_wad` is the **anchor** source and
/// `safe_price_wad` is the **primary** source. Internal code uses
/// `primary`/`anchor`; `price_components` below is the boundary that maps the
/// internal names to these ABI names.
pub struct PriceComponents {
    pub aggregator_price_wad: Option<i128>,
    pub safe_price_wad: Option<i128>,
    pub final_price_wad: i128,
    pub within_first_tolerance: bool,
    pub within_second_tolerance: bool,
}

pub fn price_components(cache: &mut Cache, asset: &Address) -> PriceComponents {
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
