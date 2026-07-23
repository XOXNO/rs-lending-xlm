//! Certora harness for `controller::oracle::price`.
//! Nondet-bounded price resolution (real logic in `controller::oracle`).

use crate::types::{AssetOracleConfig, PriceFeedRaw};
use soroban_sdk::Address;

use crate::context::Cache;
use crate::spec::summaries::token_price_summary;

pub fn token_price(cache: &mut Cache, asset: &Address) -> PriceFeedRaw {
    // Cache hit: return stored feed; miss: nondet summary.
    if let Some(feed) = cache.token_prices.get(asset.clone()) {
        return feed;
    }
    token_price_summary(cache, asset)
}

/// Config is unused under the nondet summary; same bounds as `token_price`.
pub fn resolve_with_config(
    cache: &mut Cache,
    asset: &Address,
    _config: &AssetOracleConfig,
) -> PriceFeedRaw {
    token_price_summary(cache, asset)
}
