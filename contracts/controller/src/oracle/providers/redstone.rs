#![allow(dead_code)]

// Identity validation:
// Redstone's `read_price_data_for_feed` ABI has no base/quote accessor (unlike
// Reflector's `base()`). The quote currency is implicit in `feed_id`; on-chain
// validation covers decimals, freshness, and sanity bounds only.

use common::errors::GenericError;
use common::types::{OracleProviderKind, OracleReadMode, RedStoneSourceConfig};
use soroban_sdk::{contractclient, contracttype, panic_with_error, Env, Error, String, U256};

use super::super::observation::{
    check_not_future_at, millis_to_seconds, normalize_positive_price, u256_to_i128,
    OracleObservation,
};
use crate::cache::ControllerCache;

#[contracttype]
#[derive(Clone, Debug)]
pub struct RedStonePriceData {
    pub price: U256,
    pub package_timestamp: u64,
    pub write_timestamp: u64,
}

pub(crate) const REDSTONE_DECIMALS: u32 = 8;

#[contractclient(name = "RedStonePriceFeedClient")]
pub trait RedStoneMultiFeed {
    fn read_price_data_for_feed(env: Env, feed_id: String) -> Result<RedStonePriceData, Error>;
}

pub(crate) fn read_redstone_source(
    cache: &ControllerCache,
    config: &RedStoneSourceConfig,
    required: bool,
) -> Option<OracleObservation> {
    let env = cache.env();
    let client = RedStonePriceFeedClient::new(env, &config.contract);

    let price_data = match client.try_read_price_data_for_feed(&config.feed_id) {
        Ok(Ok(price_data)) => price_data,
        _ if required => panic_with_error!(env, GenericError::InvalidTicker),
        _ => return None,
    };

    Some(observation_from_price_data(
        env,
        &price_data,
        config.decimals,
    ))
}

fn observation_from_price_data(
    env: &Env,
    price_data: &RedStonePriceData,
    decimals: u32,
) -> OracleObservation {
    let package_ts = millis_to_seconds(env, price_data.package_timestamp);
    let write_ts = millis_to_seconds(env, price_data.write_timestamp);
    let now_secs = env.ledger().timestamp();
    check_not_future_at(env, now_secs, package_ts);
    check_not_future_at(env, now_secs, write_ts);

    let raw_price = u256_to_i128(env, &price_data.price);
    OracleObservation {
        price_wad: normalize_positive_price(env, raw_price, decimals),
        raw_price,
        raw_decimals: decimals,
        observed_at: write_ts,
        published_at: Some(package_ts),
        provider: OracleProviderKind::RedStonePriceFeed,
        read_mode: OracleReadMode::Spot,
    }
}
