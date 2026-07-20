//! RedStone multi-feed client and call wrappers.

use soroban_sdk::{contractclient, contracttype, Address, Env, Error, String, Vec, U256};

#[contracttype]
#[derive(Clone, Debug)]
pub struct RedStonePriceData {
    pub price: U256,
    pub package_timestamp: u64,
    pub write_timestamp: u64,
}

pub const REDSTONE_DECIMALS: u32 = 8;

/// Wire ABI of the deployed RedStone adapter: `read_price_data` is the BULK
/// endpoint, `read_price_data_for_feed` the single-feed one. The local
/// wrapper names below do not mirror the wire names.
#[contractclient(name = "RedStonePriceFeedClient")]
#[allow(dead_code)] // Required: trait exists only for the macro to generate the client proxy.
pub trait RedStoneMultiFeed {
    fn read_price_data_for_feed(env: Env, feed_id: String) -> Result<RedStonePriceData, Error>;
    fn read_price_data(env: Env, feed_ids: Vec<String>) -> Result<Vec<RedStonePriceData>, Error>;
}

/// Single-feed read without cache. Called directly by market-config
/// validation flows (no transaction cache); the production read path also
/// calls this on a transaction-cache miss to populate its own cache.
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

/// Xoxno adapter admin/read surface beyond the shared RedStone wire ABI.
#[contractclient(name = "XoxnoOracleAdapterClient")]
#[allow(dead_code)]
pub trait XoxnoOracleAdapter {
    fn max_submission_age_seconds(env: Env) -> u64;
    fn max_stale_seconds(env: Env) -> u64;
    fn max_relative_skew_seconds(env: Env) -> u64;
}

pub fn xoxno_max_submission_age_call(env: &Env, contract: &Address) -> u64 {
    XoxnoOracleAdapterClient::new(env, contract).max_submission_age_seconds()
}
