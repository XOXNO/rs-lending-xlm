//! RedStone multi-feed client and call wrappers.

use soroban_sdk::{contractclient, contracttype, Address, Env, Error, String, Vec, U256};

use crate::cache::Cache;

#[contracttype]
#[derive(Clone, Debug)]
pub struct RedStonePriceData {
    pub price: U256,
    pub package_timestamp: u64,
    pub write_timestamp: u64,
}

pub(crate) const REDSTONE_DECIMALS: u32 = 8;

/// Wire ABI of the deployed RedStone adapter: `read_price_data` is the BULK
/// endpoint, `read_price_data_for_feed` the single-feed one. The local
/// wrapper names below do not mirror the wire names.
#[contractclient(name = "RedStonePriceFeedClient")]
#[allow(dead_code)] // Required: trait exists only for the macro to generate the client proxy.
pub trait RedStoneMultiFeed {
    fn read_price_data_for_feed(env: Env, feed_id: String) -> Result<RedStonePriceData, Error>;
    fn read_price_data(env: Env, feed_ids: Vec<String>) -> Result<Vec<RedStonePriceData>, Error>;
}

/// Reads RedStone price data, returning `None` on provider failure.
/// Served from the tx-local prefetch when `prefetch_redstone_feeds` ran.
/// A lazy uncached read also warms the same map so any later consumer of
/// this (adapter, feed_id) within the tx is a cache hit.
pub(crate) fn read_price_data(
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

/// Single-feed read without cache. Used by validation paths that
/// have no `Cache` (market config admin flows).
pub(crate) fn read_price_data_uncached(
    env: &Env,
    contract: &Address,
    feed_id: &String,
) -> Option<RedStonePriceData> {
    match RedStonePriceFeedClient::new(env, contract).try_read_price_data_for_feed(feed_id) {
        Ok(Ok(data)) => Some(data),
        _ => None,
    }
}

/// One cross-contract call for all feeds of one adapter. `None` on any
/// failure or length mismatch; callers fall back to per-feed reads.
#[cfg(not(feature = "certora"))]
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
