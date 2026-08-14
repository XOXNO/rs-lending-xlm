//! Cross-contract client trait and call helper for reading RedStone price
//! feed data from a RedStone price feed contract.

use soroban_sdk::{contractclient, contracttype, Address, Env, Error, String, Vec, U256};

/// A single RedStone price observation, as returned by a RedStone price feed
/// contract: the price value together with the package and write timestamps
/// attached to it.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedStonePriceData {
    pub price: U256,
    pub package_timestamp: u64,
    pub write_timestamp: u64,
}

/// Number of decimal places `RedStonePriceData::price` is scaled to.
pub const REDSTONE_DECIMALS: u32 = 8;

/// Client interface for a RedStone multi-feed price contract.
#[contractclient(name = "RedStonePriceFeedClient")]
#[allow(dead_code)]
pub trait RedStoneMultiFeed {
    /// Returns the latest price data for `feed_id`, or an error if the feed is
    /// unknown or unavailable.
    fn read_price_data_for_feed(env: Env, feed_id: String) -> Result<RedStonePriceData, Error>;
    /// Returns the latest price data for each id in `feed_ids`, in the same order.
    fn read_price_data(env: Env, feed_ids: Vec<String>) -> Result<Vec<RedStonePriceData>, Error>;
}

/// Reads the latest price data for `feed_id` from `contract` via
/// `read_price_data_for_feed`. Returns `None` if the call fails.
pub fn read_price_data_uncached(
    env: &Env,
    contract: &Address,
    feed_id: &String,
) -> Option<RedStonePriceData> {
    match RedStonePriceFeedClient::new(env, contract).try_read_price_data_for_feed(feed_id) {
        Ok(Ok(data)) => Some(data),
        _ => None,
    }
}
