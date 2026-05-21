//! Certora harness substitute for `controller::oracle::price`. Replaces
//! the cache-aware price reads with bounded nondet returns so the prover
//! doesn't traverse the primary/anchor composition pipeline.

use common::types::{MarketIndex, PriceFeedRaw};
use soroban_sdk::Address;

use crate::cache::ControllerCache;
use crate::spec::summaries::{token_price_summary, update_asset_index_summary};

pub fn token_price(cache: &mut ControllerCache, asset: &Address) -> PriceFeedRaw {
    token_price_summary(cache, asset)
}

pub fn update_asset_index(cache: &mut ControllerCache, asset: &Address) -> MarketIndex {
    update_asset_index_summary(cache, asset)
}
