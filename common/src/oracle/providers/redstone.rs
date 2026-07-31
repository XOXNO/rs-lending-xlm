use soroban_sdk::{contractclient, contracttype, Address, Env, Error, String, Vec, U256};

#[contracttype]
#[derive(Clone, Debug)]
pub struct RedStonePriceData {
    pub price: U256,
    pub package_timestamp: u64,
    pub write_timestamp: u64,
}

pub const REDSTONE_DECIMALS: u32 = 8;

#[contractclient(name = "RedStonePriceFeedClient")]
#[allow(dead_code)]
pub trait RedStoneMultiFeed {
    fn read_price_data_for_feed(env: Env, feed_id: String) -> Result<RedStonePriceData, Error>;
    fn read_price_data(env: Env, feed_ids: Vec<String>) -> Result<Vec<RedStonePriceData>, Error>;
}

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
