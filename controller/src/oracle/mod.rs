mod compose;
mod observation;
pub mod policy;
#[cfg(not(feature = "certora"))]
mod price;
#[cfg(feature = "certora")]
#[path = "../../../verification/certora/controller/harness/oracle_price.rs"]
mod price;
mod providers;
#[cfg(not(feature = "certora"))]
pub mod reflector;
#[cfg(feature = "certora")]
#[path = "../../../verification/certora/controller/harness/oracle_reflector.rs"]
pub mod reflector;
#[cfg(not(feature = "certora"))]
mod tolerance;
#[cfg(feature = "certora")]
#[path = "../../../verification/certora/controller/harness/oracle_tolerance.rs"]
mod tolerance;
pub(crate) mod validation;

use soroban_sdk::Address;

use crate::cache::ControllerCache;

#[allow(unused_imports)]
pub(crate) use tolerance::{calculate_final_price, is_within_anchor};

pub use price::{token_price, update_asset_index};

// Public projection of the oracle composition pipeline. Mirrors the
// internal `ResolvedOracleComponents` with stable field names suited
// for view APIs. Returned by `price_components`.
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
