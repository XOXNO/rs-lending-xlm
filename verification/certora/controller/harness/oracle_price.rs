//! Certora harness for `controller::oracle::price`.
//!
//! Under the `certora` feature this replaces the real price resolution (the
//! primary/anchor/TWAP/tolerance pipeline) with nondet-bounded returns, so
//! rules can reason about cache behavior and high-level price post-conditions
//! at low prover cost. The real logic lives in `controller::oracle`.

use common::types::{MarketIndex, PriceFeedRaw};
use soroban_sdk::Address;

use crate::cache::Cache;
use crate::spec::summaries::{token_price_summary, update_asset_index_summary};

pub fn token_price(cache: &mut Cache, asset: &Address) -> PriceFeedRaw {
    token_price_summary(cache, asset)
}

pub fn update_asset_index(cache: &mut Cache, asset: &Address) -> MarketIndex {
    update_asset_index_summary(cache, asset)
}
