//! Cache-aware RedStone read paths over the shared client wrappers.

use common::oracle::providers::redstone::{
    read_price_data_uncached, RedStonePriceData, RedStonePriceFeedClient,
};
use soroban_sdk::{Address, Env, String, Vec};

use crate::cache::Cache;

/// Reads RedStone price data, returning `None` on provider failure.
/// Uses and warms the transaction-local RedStone cache.
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

/// One cross-contract call for all feeds of one adapter. `None` on any
/// failure or length mismatch; callers fall back to per-feed reads.
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
