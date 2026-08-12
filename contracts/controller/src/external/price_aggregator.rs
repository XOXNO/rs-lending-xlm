
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
