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
    SingleCalls,
    BulkCalls,
    TruncateBulk,
    Decimals,
}

const DEFAULT_DECIMALS: u32 = 8;

#[contract]
pub struct MockRedStonePriceFeed;

impl MockRedStonePriceFeed {
    fn bump(env: &Env, key: MockKey) {
        let n: u32 = env.storage().temporary().get(&key).unwrap_or(0);
        env.storage().temporary().set(&key, &(n + 1));
    }

    fn get_feed(env: &Env, feed_id: String) -> Result<RedStonePriceData, Error> {
        env.storage()
            .temporary()
            .get(&MockKey::PriceData(feed_id))
            .ok_or_else(|| Error::from_contract_error(GenericError::InvalidTicker as u32))
    }
}

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
        // Scale the WAD input down to the mock's advertised decimals.
        let decimals = Self::decimals(env.clone());
        let price_raw = (price_wad / 10i128.pow(18 - decimals)) as u128;
        let data = RedStonePriceData {
            price: U256::from_u128(&env, price_raw),
            package_timestamp,
            write_timestamp,
        };
        env.storage()
            .temporary()
            .set(&MockKey::PriceData(feed_id), &data);
    }

    /// Overrides the advertised feed decimals (default 8, the RedStone width).
    /// Set before `set_price*`: prices are scaled at write time.
    pub fn set_decimals(env: Env, decimals: u32) {
        env.storage().temporary().set(&MockKey::Decimals, &decimals);
    }

    pub fn set_bulk_truncate(env: Env, truncate: bool) {
        env.storage()
            .temporary()
            .set(&MockKey::TruncateBulk, &truncate);
    }

    /// SEP-40-style decimals probe, as served by the XOXNO adapter.
    pub fn decimals(env: Env) -> u32 {
        env.storage()
            .temporary()
            .get(&MockKey::Decimals)
            .unwrap_or(DEFAULT_DECIMALS)
    }

    pub fn read_price_data_for_feed(env: Env, feed_id: String) -> Result<RedStonePriceData, Error> {
        Self::bump(&env, MockKey::SingleCalls);
        Self::get_feed(&env, feed_id)
    }

    pub fn read_price_data(
        env: Env,
        feed_ids: Vec<String>,
    ) -> Result<Vec<RedStonePriceData>, Error> {
        Self::bump(&env, MockKey::BulkCalls);
        let mut values = Vec::new(&env);
        for feed_id in feed_ids.iter() {
            values.push_back(Self::get_feed(&env, feed_id)?);
        }
        if env
            .storage()
            .temporary()
            .get(&MockKey::TruncateBulk)
            .unwrap_or(false)
        {
            let _ = values.pop_back();
        }
        Ok(values)
    }

    pub fn read_prices(env: Env, feed_ids: Vec<String>) -> Result<Vec<U256>, Error> {
        let mut prices = Vec::new(&env);
        for feed_id in feed_ids.iter() {
            prices.push_back(Self::get_feed(&env, feed_id)?.price);
        }
        Ok(prices)
    }

    pub fn read_timestamp(env: Env, feed_id: String) -> Result<u64, Error> {
        Ok(Self::get_feed(&env, feed_id)?.package_timestamp)
    }

    pub fn single_calls(env: Env) -> u32 {
        env.storage()
            .temporary()
            .get(&MockKey::SingleCalls)
            .unwrap_or(0)
    }

    pub fn bulk_calls(env: Env) -> u32 {
        env.storage()
            .temporary()
            .get(&MockKey::BulkCalls)
            .unwrap_or(0)
    }
}
