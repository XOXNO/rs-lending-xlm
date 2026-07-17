//! Read entrypoints: the RedStone `RedStoneMultiFeed` ABI and the SEP-40
//! `PriceFeedTrait` shape. SEP-40 reads delegate to the RedStone reads via
//! plain function calls (same contract, no cross-contract overhead).

use common::constants::MS_PER_SECOND;
use common::oracle::observation::{millis_to_seconds, u256_to_i128};
use common::oracle::providers::redstone::{RedStonePriceData, REDSTONE_DECIMALS};
use common::oracle::providers::reflector::{ReflectorAsset, ReflectorPriceData};

use soroban_sdk::{contractimpl, Env, String, Symbol, Vec};

use crate::aggregation::MAX_HISTORY_LEN;
use crate::storage::{
    load_all_assets, load_feed_id, load_max_stale_seconds, load_resolution, renew_persistent_key,
    DataKey,
};
use crate::{Error, XoxnoOracle, XoxnoOracleArgs, XoxnoOracleClient};

// RedStone ABI — mirrors `RedStoneMultiFeed` exactly.
#[contractimpl]
impl XoxnoOracle {
    /// # Errors
    /// * `NoDataForFeed`
    /// * `StaleData` - exceeds MaxStaleSeconds
    pub fn read_price_data_for_feed(env: Env, feed_id: String) -> Result<RedStonePriceData, Error> {
        let key = DataKey::CurrentAggregate(feed_id.clone());
        let aggregate: RedStonePriceData = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::NoDataForFeed)?;

        renew_persistent_key(&env, &key);

        let max_stale = load_max_stale_seconds(&env);
        // `write_timestamp` is milliseconds, the ledger clock is seconds;
        // saturate so a write at/after `now` reads as fresh.
        let age_seconds = env
            .ledger()
            .timestamp()
            .saturating_sub(aggregate.write_timestamp / MS_PER_SECOND);
        if age_seconds > max_stale {
            return Err(Error::StaleData);
        }
        Ok(aggregate)
    }

    /// All-or-nothing bulk; first missing/stale fails the whole call.
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

    /// Newest-first history cap for SEP-40 `price`/`prices`.
    ///
    /// # Errors
    /// * `NoDataForFeed`
    pub fn read_price_history(
        env: Env,
        feed_id: String,
        limit: u32,
    ) -> Result<Vec<RedStonePriceData>, Error> {
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
            newest_first.push_back(
                history
                    .get(history.len() - 1 - i)
                    .expect("invariant: i < take <= history.len()"),
            );
        }
        Ok(newest_first)
    }
}

// SEP-40 / Reflector ABI.
#[contractimpl]
impl XoxnoOracle {
    pub fn base(env: Env) -> ReflectorAsset {
        ReflectorAsset::Other(Symbol::new(&env, "USD"))
    }

    pub fn decimals(_env: Env) -> u32 {
        REDSTONE_DECIMALS
    }

    pub fn resolution(env: Env) -> u32 {
        load_resolution(&env)
    }

    pub fn assets(env: Env) -> Vec<ReflectorAsset> {
        load_all_assets(&env)
    }

    /// `None` when unmapped/missing/stale (SEP-40 soft-fail).
    pub fn lastprice(env: Env, asset: ReflectorAsset) -> Option<ReflectorPriceData> {
        let feed_id = load_feed_id(&env, &asset)?;
        let data = Self::read_price_data_for_feed(env.clone(), feed_id).ok()?;
        Some(to_reflector_price_data(&env, &data))
    }

    /// Closest observation at or before `timestamp` (package time, not write time).
    pub fn price(env: Env, asset: ReflectorAsset, timestamp: u64) -> Option<ReflectorPriceData> {
        let feed_id = load_feed_id(&env, &asset)?;
        let history = Self::read_price_history(env.clone(), feed_id, MAX_HISTORY_LEN).ok()?;

        // Select on `package_timestamp` (the exposed observation time), which
        // is non-monotonic across recomputes — scan the whole window for the
        // greatest qualifying observation rather than trusting position.
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

// SEP-40 timestamp = observation (`package_timestamp`), never write time:
// write time would accept stale aggregates the RedStone path rejects.
fn to_reflector_price_data(env: &Env, data: &RedStonePriceData) -> ReflectorPriceData {
    let price = u256_to_i128(env, &data.price);
    let timestamp = millis_to_seconds(data.package_timestamp);
    ReflectorPriceData { price, timestamp }
}
