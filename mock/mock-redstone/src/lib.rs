//! RedStone price-feed mock for live testnet runs.
//! Stores 8-decimal prices; setters accept USD WAD.

#![no_std]

use soroban_sdk::{
    assert_with_error, contract, contracterror, contractimpl, contracttype, Env, String, Vec, U256,
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum MockRedStoneError {
    FeedNotSet = 1,
    InvalidPrice = 2,
    TimestampOverflow = 3,
}

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

const WAD_TO_8_DECIMALS: i128 = 10_000_000_000;
const SECONDS_TO_MS: u64 = 1000;

#[contract]
pub struct MockRedStonePriceFeed;

#[contractimpl]
impl MockRedStonePriceFeed {
    /// Sets feed price at current ledger time.
    pub fn set_price(env: Env, feed_id: String, price_wad: i128) {
        let now_ms = env
            .ledger()
            .timestamp()
            .checked_mul(SECONDS_TO_MS)
            .unwrap_or_else(|| {
                soroban_sdk::panic_with_error!(&env, MockRedStoneError::TimestampOverflow)
            });
        Self::set_price_data(env, feed_id, price_wad, now_ms, now_ms);
    }

    /// Sets feed price with explicit millisecond timestamps.
    pub fn set_price_data(
        env: Env,
        feed_id: String,
        price_wad: i128,
        package_timestamp: u64,
        write_timestamp: u64,
    ) {
        assert_with_error!(&env, price_wad >= 0, MockRedStoneError::InvalidPrice);
        let price_8 = (price_wad / WAD_TO_8_DECIMALS) as u128;
        let data = RedStonePriceData {
            price: U256::from_u128(&env, price_8),
            package_timestamp,
            write_timestamp,
        };
        env.storage()
            .persistent()
            .set(&MockKey::PriceData(feed_id), &data);
    }

    pub fn read_price_data_for_feed(
        env: Env,
        feed_id: String,
    ) -> Result<RedStonePriceData, MockRedStoneError> {
        env.storage()
            .persistent()
            .get(&MockKey::PriceData(feed_id))
            .ok_or(MockRedStoneError::FeedNotSet)
    }

    pub fn read_price_data(
        env: Env,
        feed_ids: Vec<String>,
    ) -> Result<Vec<RedStonePriceData>, MockRedStoneError> {
        let mut values = Vec::new(&env);
        for feed_id in feed_ids.iter() {
            values.push_back(Self::read_price_data_for_feed(env.clone(), feed_id)?);
        }
        Ok(values)
    }

    pub fn read_prices(env: Env, feed_ids: Vec<String>) -> Result<Vec<U256>, MockRedStoneError> {
        let mut prices = Vec::new(&env);
        for data in Self::read_price_data(env, feed_ids)?.iter() {
            prices.push_back(data.price);
        }
        Ok(prices)
    }

    pub fn read_timestamp(env: Env, feed_id: String) -> Result<u64, MockRedStoneError> {
        Ok(Self::read_price_data_for_feed(env, feed_id)?.package_timestamp)
    }
}
