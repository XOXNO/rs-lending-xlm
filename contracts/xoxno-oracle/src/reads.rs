//! Read-only query entry points for the oracle: RedStone-shaped feed price
//! and history lookups, configuration getters, and a Reflector-compatible
//! asset price API built on top of the feed data.

use common::constants::MS_PER_SECOND;
use common::oracle::observation::{millis_to_seconds, u256_to_i128};
use common::oracle::providers::redstone::{RedStonePriceData, REDSTONE_DECIMALS};
use common::oracle::providers::reflector::{ReflectorAsset, ReflectorPriceData};

use soroban_sdk::{contractimpl, Env, String, Symbol, Vec};

use crate::aggregation::MAX_HISTORY_LEN;
use crate::storage::{
    load_all_assets, load_feed_id, load_max_relative_skew, load_max_stale_seconds,
    load_max_submission_age, load_resolution, renew_persistent_key, DataKey,
};
use crate::{Error, XoxnoOracle, XoxnoOracleArgs, XoxnoOracleClient};

#[contractimpl]
impl XoxnoOracle {
    /// Returns the current aggregate for `feed_id`. Fails with `NoDataForFeed`
    /// if none is stored, or `StaleData` if `now - write_timestamp` (seconds;
    /// timestamps stored in ms) exceeds `max_stale_seconds`. Package timestamp
    /// is not used for this check.
    pub fn read_price_data_for_feed(env: Env, feed_id: String) -> Result<RedStonePriceData, Error> {
        let key = DataKey::CurrentAggregate(feed_id.clone());
        let aggregate: RedStonePriceData = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::NoDataForFeed)?;

        renew_persistent_key(&env, &key);

        let max_stale = load_max_stale_seconds(&env);

        let age_seconds = env
            .ledger()
            .timestamp()
            .saturating_sub(aggregate.write_timestamp / MS_PER_SECOND);
        if age_seconds > max_stale {
            return Err(Error::StaleData);
        }
        Ok(aggregate)
    }

    /// Returns the current aggregate price for each feed in `feed_ids`, in
    /// the same order. Fails on the first feed that returns an error from
    /// `read_price_data_for_feed`.
    pub fn read_price_data(
        env: Env,
        feed_ids: Vec<String>,
    ) -> Result<Vec<RedStonePriceData>, Error> {
        let mut results = Vec::new(&env);
        for feed_id in feed_ids.iter() {
            results.push_back(Self::read_price_data_for_feed(env.clone(), feed_id)?);
        }
        Ok(results)
    }

    /// Returns up to `limit` history entries for `feed_id`, newest first.
    /// Fails with `NoDataForFeed` if the feed has no current aggregate, is
    /// stale, or has an empty history.
    pub fn read_price_history(
        env: Env,
        feed_id: String,
        limit: u32,
    ) -> Result<Vec<RedStonePriceData>, Error> {
        Self::read_price_data_for_feed(env.clone(), feed_id.clone())?;

        let key = DataKey::History(feed_id.clone());
        let history: Vec<RedStonePriceData> = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::NoDataForFeed)?;
        if history.is_empty() {
            return Err(Error::NoDataForFeed);
        }
        renew_persistent_key(&env, &key);

        let take = core::cmp::min(limit, history.len());
        let mut newest_first = Vec::new(&env);
        for i in 0..take {
            newest_first.push_back(history.get_unchecked(history.len() - 1 - i));
        }
        Ok(newest_first)
    }

    /// Returns the configured maximum submission age, in seconds.
    pub fn max_submission_age_seconds(env: Env) -> u64 {
        load_max_submission_age(&env)
    }

    /// Returns the configured maximum staleness, in seconds, for aggregate
    /// reads.
    pub fn max_stale_seconds(env: Env) -> u64 {
        load_max_stale_seconds(&env)
    }

    /// Returns the configured maximum relative timestamp skew, in seconds,
    /// between clustered submissions.
    pub fn max_relative_skew_seconds(env: Env) -> u64 {
        load_max_relative_skew(&env)
    }
}

#[contractimpl]
impl XoxnoOracle {
    /// Returns the quote asset for all prices reported by this contract:
    /// `ReflectorAsset::Other("USD")`.
    pub fn base(env: Env) -> ReflectorAsset {
        ReflectorAsset::Other(Symbol::new(&env, "USD"))
    }

    /// Returns the number of decimals used for reported prices.
    pub fn decimals(_env: Env) -> u32 {
        REDSTONE_DECIMALS
    }

    /// Returns the configured price resolution, in seconds.
    pub fn resolution(env: Env) -> u32 {
        load_resolution(&env)
    }

    /// Returns every `ReflectorAsset` currently mapped to a feed.
    pub fn assets(env: Env) -> Vec<ReflectorAsset> {
        load_all_assets(&env)
    }

    /// Returns the latest price for `asset`, converted to `ReflectorPriceData`.
    /// Returns `None` if `asset` has no feed mapping or the feed's aggregate
    /// is unavailable or stale.
    pub fn lastprice(env: Env, asset: ReflectorAsset) -> Option<ReflectorPriceData> {
        let feed_id = load_feed_id(&env, &asset)?;
        let data = Self::read_price_data_for_feed(env.clone(), feed_id).ok()?;
        Some(to_reflector_price_data(&env, &data))
    }

    /// Returns the history entry for `asset` with the newest package
    /// timestamp at or before `timestamp`, converted to `ReflectorPriceData`.
    /// Returns `None` if `asset` has no feed mapping, the feed's history is
    /// unavailable, or no entry satisfies the timestamp bound.
    pub fn price(env: Env, asset: ReflectorAsset, timestamp: u64) -> Option<ReflectorPriceData> {
        let feed_id = load_feed_id(&env, &asset)?;
        let history = Self::read_price_history(env.clone(), feed_id, MAX_HISTORY_LEN).ok()?;

        let mut best: Option<RedStonePriceData> = None;
        for entry in history.iter() {
            if millis_to_seconds(entry.package_timestamp) > timestamp {
                continue;
            }
            let closer = match &best {
                Some(b) => entry.package_timestamp > b.package_timestamp,
                None => true,
            };
            if closer {
                best = Some(entry);
            }
        }
        best.map(|entry| to_reflector_price_data(&env, &entry))
    }

    /// Returns up to `records` history entries for `asset`, newest first,
    /// converted to `ReflectorPriceData`. Returns `None` if `asset` has no
    /// feed mapping or its history is empty or unavailable.
    pub fn prices(
        env: Env,
        asset: ReflectorAsset,
        records: u32,
    ) -> Option<Vec<ReflectorPriceData>> {
        let feed_id = load_feed_id(&env, &asset)?;
        let history = Self::read_price_history(env.clone(), feed_id, records).ok()?;
        if history.is_empty() {
            return None;
        }
        let mut out = Vec::new(&env);
        for entry in history.iter() {
            out.push_back(to_reflector_price_data(&env, &entry));
        }
        Some(out)
    }
}

/// Converts a `RedStonePriceData` entry into `ReflectorPriceData`, scaling
/// the U256 price to i128 and the millisecond package timestamp to seconds.
fn to_reflector_price_data(env: &Env, data: &RedStonePriceData) -> ReflectorPriceData {
    let price = u256_to_i128(env, &data.price);
    let timestamp = millis_to_seconds(data.package_timestamp);
    ReflectorPriceData { price, timestamp }
}
