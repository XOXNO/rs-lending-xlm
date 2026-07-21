//! Cross-contract client for the price-aggregator (the oracle authority).
//! One `fetch_prices` call per priced flow resolves every asset the flow needs;
//! read-only views use the per-asset helpers.

use common::types::PriceFeedRaw;
use price_aggregator_interface::PriceAggregatorClient;
use soroban_sdk::{Address, Env, Map, Vec};

use crate::storage;

/// Bulk-resolves every asset a flow prices in one cross-contract call.
pub(crate) fn fetch_prices(env: &Env, assets: &Vec<Address>) -> Map<Address, PriceFeedRaw> {
    let aggregator = storage::get_price_aggregator(env);
    PriceAggregatorClient::new(env, &aggregator).prices(assets)
}

/// Single token-rooted price (read-only views only).
pub(crate) fn fetch_price(env: &Env, asset: &Address) -> PriceFeedRaw {
    let aggregator = storage::get_price_aggregator(env);
    PriceAggregatorClient::new(env, &aggregator).price(asset)
}

/// `(final, safe, aggregator)` USD-WAD price triple (read-only views only).
pub(crate) fn fetch_price_components(env: &Env, asset: &Address) -> (i128, i128, i128) {
    let aggregator = storage::get_price_aggregator(env);
    PriceAggregatorClient::new(env, &aggregator).price_components(asset)
}

/// Whether `asset` has a token-rooted oracle configured (read-only views only).
pub(crate) fn is_asset_priceable(env: &Env, asset: &Address) -> bool {
    let aggregator = storage::get_price_aggregator(env);
    PriceAggregatorClient::new(env, &aggregator)
        .get_asset_oracle(asset)
        .is_some()
}
