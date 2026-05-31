//! Minimal RedStone multi-feed adapter mock for oracle V2 integration tests.

use common::errors::GenericError;
use soroban_sdk::{contract, contractimpl, contracttype, Env, Error, String, Vec, U256};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedStonePriceData {
    pub price: U256,
    pub package_timestamp: u64,
    pub write_timestamp: u64,
}

#[contracttype]
pub enum MockKey {
    PriceData(String),
}

#[contract]
pub struct MockRedStonePriceFeed;

#[contractimpl]
impl MockRedStonePriceFeed {
    pub fn set_price(env: Env, feed_id: String, price_wad: i128) {
        let now_ms = env.ledger().timestamp() * 1000;
        Self::set_price_data(env, feed_id, price_wad, now_ms, now_ms);
    }

    pub fn set_price_data(
        env: Env,
        feed_id: String,
        price_wad: i128,
        package_timestamp: u64,
        write_timestamp: u64,
    ) {
        let price_8 = (price_wad / 10_000_000_000) as u128;
        let data = RedStonePriceData {
            price: U256::from_u128(&env, price_8),
            package_timestamp,
            write_timestamp,
        };
        env.storage()
            .temporary()
            .set(&MockKey::PriceData(feed_id), &data);
    }

    pub fn read_price_data_for_feed(env: Env, feed_id: String) -> Result<RedStonePriceData, Error> {
        env.storage()
            .temporary()
            .get(&MockKey::PriceData(feed_id))
            .ok_or_else(|| Error::from_contract_error(GenericError::InvalidTicker as u32))
    }

    pub fn read_price_data(
        env: Env,
        feed_ids: Vec<String>,
    ) -> Result<Vec<RedStonePriceData>, Error> {
        let mut values = Vec::new(&env);
        for feed_id in feed_ids.iter() {
            values.push_back(Self::read_price_data_for_feed(env.clone(), feed_id)?);
        }
        Ok(values)
    }

    pub fn read_prices(env: Env, feed_ids: Vec<String>) -> Result<Vec<U256>, Error> {
        let mut prices = Vec::new(&env);
        for data in Self::read_price_data(env, feed_ids)?.iter() {
            prices.push_back(data.price);
        }
        Ok(prices)
    }

    pub fn read_timestamp(env: Env, feed_id: String) -> Result<u64, Error> {
        Ok(Self::read_price_data_for_feed(env, feed_id)?.package_timestamp)
    }
}
