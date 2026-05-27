//! Oracle price resolution subsystem.
//!
//! This module is responsible for fetching, validating, composing, and
//! tolerancing prices from multiple external oracle providers (primarily
// Reflector SEP-40 and RedStone) to produce safe, fresh USD prices for
// the lending protocol.
//!
//! Key design principles (production-grade):
//! - Provider client surfaces live under `providers/*/client.rs`.
//! - Consumption logic (Spot/TWAP, fallback, mapping) lives with the provider.
//! - Validation is split: pure config/shape checks vs live probing.
//! - All cross-contract oracle calls go through thin, harness-friendly wrappers.
//! - Certora harnesses replace expensive paths while preserving security invariants.

mod compose;
mod observation;
pub mod policy;
#[cfg(not(feature = "certora"))]
mod price;
#[cfg(feature = "certora")]
#[path = "../../../../verification/certora/controller/harness/oracle_price.rs"]
mod price; // Full module replacement (delegates to summaries) to bound prover cost on the full primary/anchor/compose/TWAP pipeline.
pub(crate) mod providers;
#[cfg(not(feature = "certora"))]
pub(crate) mod tolerance;
#[cfg(feature = "certora")]
#[path = "../../../../verification/certora/controller/harness/oracle_tolerance.rs"]
pub(crate) mod tolerance; // Full module replacement — the is_within_anchor ratio math (I256 + BPS) is expensive; harness preserves control flow + panic paths with nondet decision. Production tolerance.rs kept clean/pure per its module doc.
pub(crate) mod validation;

use soroban_sdk::Address;

use crate::cache::ControllerCache;

// Re-exported so that Certora spec rules (compiled only under the certora
// feature) can reach the functions via `crate::oracle::*`.
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
