//! Multi-feed adapter provider (RedStone + Xoxno wire ABI): reads an adapter
//! feed (transaction-cache warmed) into an `OracleObservation`. Feeds are USD
//! by construction.

#[cfg(not(feature = "certora"))]
use common::oracle::providers::redstone::RedStonePriceFeedClient;
use common::oracle::providers::redstone::{read_price_data_uncached, RedStonePriceData};
use common::types::RedStoneSourceConfig;
use soroban_sdk::{Address, String};
#[cfg(not(feature = "certora"))]
use soroban_sdk::{Env, Vec};

use crate::context::ResolutionContext;
use crate::observation::OracleObservation;

pub(crate) fn read_multi_feed_source(
    cache: &mut ResolutionContext,
    config: &RedStoneSourceConfig,
) -> Option<OracleObservation> {
    let env = cache.env().clone();
    let now_secs = cache.ledger_timestamp_secs();
    let price_data = read_price_data(cache, &config.contract, &config.feed_id)?;
    Some(OracleObservation::from_multi_feed(
        &env,
        now_secs,
        &price_data,
        config.decimals,
    ))
}

fn read_price_data(
    cache: &mut ResolutionContext,
    contract: &Address,
    feed_id: &String,
) -> Option<RedStonePriceData> {
    if let Some(data) = cache.get_bulk_feed(contract, feed_id) {
        return Some(data);
    }
    let env = cache.env().clone();
    let data = read_price_data_uncached(&env, contract, feed_id)?;
    cache.set_bulk_feed(contract, feed_id, data.clone());
    Some(data)
}

/// One cross-contract call for all multi-feed adapter feeds. `None` on any
/// failure or length mismatch; callers fall back to per-feed reads. Used by
/// `warm_multi_feed_adapters`.
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
