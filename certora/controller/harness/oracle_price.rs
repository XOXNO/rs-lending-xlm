//! Certora harness for `controller::oracle::price`.
//!
//! Under the `certora` feature this replaces the real price resolution (the
//! primary/anchor/TWAP/tolerance pipeline) with nondet-bounded returns, so
//! rules can reason about cache behavior and high-level price post-conditions
//! at low prover cost. The real logic lives in `controller::oracle`.

use crate::types::{MarketOracleConfig, PriceFeedRaw};
use soroban_sdk::Address;

use crate::cache::Cache;
use crate::spec::summaries::token_price_summary;

pub fn token_price(cache: &mut Cache, asset: &Address) -> PriceFeedRaw {
    // Cache-hit returns the stored feed unchanged (mirrors production
    // `oracle::price::token_price`); only a cache-miss resolves nondet.
    if let Some(feed) = cache.prices_cache.get(asset.clone()) {
        return feed;
    }
    token_price_summary(cache, asset)
}

/// Mirrors production `price_with_config`: the resolved config is irrelevant
/// under the nondet summary, so the per-spoke override path collapses to the
/// same bounded price as `token_price`.
pub fn price_with_config(
    cache: &mut Cache,
    asset: &Address,
    _config: &MarketOracleConfig,
) -> PriceFeedRaw {
    token_price_summary(cache, asset)
}
