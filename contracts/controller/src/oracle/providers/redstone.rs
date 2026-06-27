//! RedStone Price Feed provider: reads an adapter feed (transaction-cache warmed)
//! into an `OracleObservation`. RedStone feeds are USD by construction.

use common::oracle::providers::redstone::{
    read_price_data_uncached, RedStonePriceData, RedStonePriceFeedClient,
};
use controller_interface::types::RedStoneSourceConfig;
use soroban_sdk::{Address, Env, String, Vec};

use crate::cache::Cache;
use crate::oracle::observation::{redstone_observation_from_price_data, OracleObservation};

pub(crate) fn read_redstone_source(
    cache: &mut Cache,
    config: &RedStoneSourceConfig,
) -> Option<OracleObservation> {
    let env = cache.env().clone();
    let price_data = read_price_data(cache, &config.contract, &config.feed_id)?;
    Some(redstone_observation_from_price_data(
        &env,
        &price_data,
        config.decimals,
    ))
}

/// Reads RedStone price data, returning `None` on provider failure.
/// Uses and warms the transaction-local RedStone cache.
fn read_price_data(
    cache: &mut Cache,
    contract: &Address,
    feed_id: &String,
) -> Option<RedStonePriceData> {
    if let Some(data) = cache.get_redstone_prefetch(contract, feed_id) {
        return Some(data);
    }
    let env = cache.env().clone();
    let data = read_price_data_uncached(&env, contract, feed_id)?;
    cache.set_redstone_prefetch(contract, feed_id, data.clone());
    Some(data)
}

/// One cross-contract call for all feeds of one adapter. `None` on any failure
/// or length mismatch; callers fall back to per-feed reads. Used by `prefetch`.
pub(crate) fn read_price_data_bulk(
    env: &Env,
    contract: &Address,
    feed_ids: &Vec<String>,
) -> Option<Vec<RedStonePriceData>> {
    match RedStonePriceFeedClient::new(env, contract).try_read_price_data(feed_ids) {
        Ok(Ok(data)) if data.len() == feed_ids.len() => Some(data),
        _ => None,
    }
}
