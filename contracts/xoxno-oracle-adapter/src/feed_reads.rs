//! Reads — mirror `RedStoneMultiFeed`'s ABI exactly.

use common::constants::MS_PER_SECOND;
use common::oracle::providers::redstone::RedStonePriceData;
use soroban_sdk::{contractimpl, Env, String, Vec};

use crate::storage::{load_max_stale_seconds, DataKey};
use crate::{Error, XoxnoOracle, XoxnoOracleArgs, XoxnoOracleClient};

#[contractimpl]
impl XoxnoOracle {
    /// Returns the cached aggregate for `feed_id`.
    ///
    /// # Errors
    /// * `NoDataForFeed` - no aggregate has been computed for `feed_id` yet.
    /// * `StaleData` - the cached aggregate exceeds `MaxStaleSeconds`.
    pub fn read_price_data_for_feed(env: Env, feed_id: String) -> Result<RedStonePriceData, Error> {
        let aggregate: RedStonePriceData = env
            .storage()
            .persistent()
            .get(&DataKey::CurrentAggregate(feed_id))
            .ok_or(Error::NoDataForFeed)?;

        let max_stale = load_max_stale_seconds(&env);
        // `write_timestamp` is milliseconds; `ledger().timestamp()` is
        // seconds. Saturate rather than subtract directly: a `write_timestamp`
        // at or after `now` (clock skew, or same-second write) must read as
        // "not stale", not underflow.
        let age_seconds = env
            .ledger()
            .timestamp()
            .saturating_sub(aggregate.write_timestamp / MS_PER_SECOND);
        if age_seconds > max_stale {
            return Err(Error::StaleData);
        }
        Ok(aggregate)
    }

    /// Bulk read: all-or-nothing. Propagates the first missing/stale feed
    /// instead of returning partial results, matching the real RedStone
    /// Stellar adapter's bulk semantics and the controller's bulk-fallback
    /// expectations.
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

    /// Returns up to `limit` most-recent aggregates for `feed_id`, newest
    /// first. Not part of `RedStoneMultiFeed`; backs the SEP-40
    /// `price()`/`prices()` reads below.
    ///
    /// # Errors
    /// * `NoDataForFeed` - no history exists for `feed_id`.
    pub fn read_price_history(
        env: Env,
        feed_id: String,
        limit: u32,
    ) -> Result<Vec<RedStonePriceData>, Error> {
        let history: Vec<RedStonePriceData> = env
            .storage()
            .persistent()
            .get(&DataKey::History(feed_id))
            .ok_or(Error::NoDataForFeed)?;
        if history.is_empty() {
            return Err(Error::NoDataForFeed);
        }

        let take = core::cmp::min(limit, history.len());
        let mut newest_first = Vec::new(&env);
        for i in 0..take {
            // safe: i < take <= history.len(), so history.len()-1-i is in bounds.
            newest_first.push_back(history.get(history.len() - 1 - i).unwrap());
        }
        Ok(newest_first)
    }
}
