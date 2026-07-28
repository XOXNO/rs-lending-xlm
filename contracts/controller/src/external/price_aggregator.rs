//! Cross-contract client for the price-aggregator.
//! Lifts token addresses to [`PriceKey::Token`] then rekeys results by Address
//! so the controller cache stays address-keyed.

use common::errors::OracleError;
use common::types::{PriceFeedRaw, PriceKey, PriceStatus};
use price_aggregator_interface::PriceAggregatorClient;
use soroban_sdk::{panic_with_error, Address, Env, Map, Vec};

use crate::storage;

fn token_keys(env: &Env, assets: &Vec<Address>) -> Vec<PriceKey> {
    let mut keys = Vec::new(env);
    for asset in assets.iter() {
        keys.push_back(PriceKey::Token(asset));
    }
    keys
}

/// Bulk-resolves every asset a flow prices in one cross-contract call.
///
/// Hard path: every requested asset must appear in the return map. A missing
/// key panics immediately (`OracleNotConfigured`) rather than leaving a hole
/// for a later cache miss with a less clear site.
pub(crate) fn fetch_prices(env: &Env, assets: &Vec<Address>) -> Map<Address, PriceFeedRaw> {
    let aggregator = storage::get_price_aggregator(env);
    let keyed = PriceAggregatorClient::new(env, &aggregator).prices(&token_keys(env, assets));
    let mut out = Map::new(env);
    for asset in assets.iter() {
        match keyed.get(PriceKey::Token(asset.clone())) {
            Some(feed) => out.set(asset, feed),
            None => panic_with_error!(env, OracleError::OracleNotConfigured),
        }
    }
    out
}

/// Bulk soft oracle statuses for multi-asset views.
///
/// Soft path never panics on a missing key: inserts [`PriceStatus::unusable`]
/// so diagnostic maps stay complete for every requested asset.
pub(crate) fn fetch_prices_status(env: &Env, assets: &Vec<Address>) -> Map<Address, PriceStatus> {
    let aggregator = storage::get_price_aggregator(env);
    let keyed = PriceAggregatorClient::new(env, &aggregator).quotes(&token_keys(env, assets));
    let mut out = Map::new(env);
    for asset in assets.iter() {
        match keyed.get(PriceKey::Token(asset.clone())) {
            Some(status) => out.set(asset, status),
            None => out.set(asset, PriceStatus::unusable()),
        }
    }
    out
}
