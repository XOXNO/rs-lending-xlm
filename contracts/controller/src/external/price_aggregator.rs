//! Cross-contract client for the price-aggregator (the oracle authority).
//! Priced write paths and multi-asset views use one bulk call per flow.

use common::types::{PriceFeedRaw, PriceStatus};
use price_aggregator_interface::PriceAggregatorClient;
use soroban_sdk::{Address, Env, Map, Vec};

use crate::storage;

/// Bulk-resolves every asset a flow prices in one cross-contract call.
pub(crate) fn fetch_prices(env: &Env, assets: &Vec<Address>) -> Map<Address, PriceFeedRaw> {
    let aggregator = storage::get_price_aggregator(env);
    PriceAggregatorClient::new(env, &aggregator).prices(assets)
}

/// Bulk soft oracle statuses for multi-asset views (flags, no stale/deviation trap).
pub(crate) fn fetch_prices_status(env: &Env, assets: &Vec<Address>) -> Map<Address, PriceStatus> {
    let aggregator = storage::get_price_aggregator(env);
    PriceAggregatorClient::new(env, &aggregator).prices_status(assets)
}
