//! Read-only query entry points for the oracle. The module has RedStone-shaped
//! feed price and history lookups, configuration getters, and a
//! Reflector-compatible asset price API built on the feed data.

use common::constants::MS_PER_SECOND;
use common::oracle::observation::{millis_to_seconds, u256_to_i128};
use common::oracle::providers::redstone::{RedStonePriceData, REDSTONE_DECIMALS};
use common::oracle::providers::reflector::{ReflectorAsset, ReflectorPriceData};

use soroban_sdk::{contractimpl, Env, String, Symbol, Vec};

use crate::aggregation::{bucket_start_seconds, MAX_HISTORY_LEN};
use crate::storage::{
    load_aggregate, load_all_assets, load_all_feeds, load_feed_id, load_history,
    load_max_cluster_spread_bps, load_max_relative_skew, load_max_stale_seconds,
    load_max_submission_age, load_resolution, renew_history,
};
use crate::{Error, XoxnoOracle, XoxnoOracleArgs, XoxnoOracleClient};

#[contractimpl]
impl XoxnoOracle {
    /// Returns the current aggregate for `feed_id`. Fails with `NoDataForFeed`
    /// if no aggregate is stored. Fails with `StaleData` if
    /// `now - write_timestamp` exceeds `max_stale_seconds`. The check converts
    /// the stored millisecond write timestamp to seconds. It does not use the
    /// package timestamp.
    pub fn read_price_data_for_feed(env: Env, feed_id: String) -> Result<RedStonePriceData, Error> {
        let aggregate = load_aggregate(&env, &feed_id).ok_or(Error::NoDataForFeed)?;

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
    /// Fails with `NoDataForFeed` if the feed has no current aggregate or no
    /// history entries, and with `StaleData` if the current aggregate is stale.
    pub fn read_price_history(
        env: Env,
        feed_id: String,
        limit: u32,
    ) -> Result<Vec<RedStonePriceData>, Error> {
        Self::read_price_data_for_feed(env.clone(), feed_id.clone())?;

        let history = load_history(&env, &feed_id).ok_or(Error::NoDataForFeed)?;
        if history.is_empty() {
            return Err(Error::NoDataForFeed);
        }
        renew_history(&env, &feed_id);

        let take = limit.min(history.len());
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

    /// Returns the effective maximum relative timestamp skew, in seconds,
    /// between clustered submissions. The getter clamps the configured value to
    /// the maximum submission age.
    pub fn max_relative_skew_seconds(env: Env) -> u64 {
        load_max_relative_skew(&env)
    }

    /// Returns the configured maximum price spread, in basis points, of a
    /// cluster smaller than `2 * (signers - threshold) + 1` entries.
    pub fn max_cluster_spread_bps(env: Env) -> u32 {
        load_max_cluster_spread_bps(&env)
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

    /// Returns every registered feed id, including feeds with no asset
    /// mapping. Use this to enumerate the ids to pass to `recompute_feeds`
    /// after a configuration change.
    pub fn feeds(env: Env) -> Vec<String> {
        load_all_feeds(&env)
    }

    /// Returns the latest price for `asset`, converted to `ReflectorPriceData`.
    /// Returns `None` if `asset` has no feed mapping or the feed's aggregate
    /// is unavailable or stale.
    pub fn lastprice(env: Env, asset: ReflectorAsset) -> Option<ReflectorPriceData> {
        let feed_id = load_feed_id(&env, &asset)?;
        let data = Self::read_price_data_for_feed(env.clone(), feed_id).ok()?;
        Some(to_reflector_price_data(&env, &data))
    }

    /// Returns the newest history round of `asset` written at or before
    /// `timestamp` (seconds). See [`history_rounds`] for what a round holds.
    /// The answer never holds a price written after `timestamp`, and its
    /// timestamp is at or before `timestamp`.
    /// Returns `None` if `asset` has no feed mapping. Returns `None` if the feed
    /// has no current aggregate or that aggregate is stale. Returns `None` if the
    /// feed's history is unavailable or no round satisfies the timestamp bound.
    pub fn price(env: Env, asset: ReflectorAsset, timestamp: u64) -> Option<ReflectorPriceData> {
        history_rounds(&env, &asset, timestamp, 1)?.first()
    }

    /// Returns up to `records` history rounds of `asset`, newest first. See
    /// [`history_rounds`] for what a round holds. Returns `None` if `asset`
    /// has no feed mapping. Returns `None` if the feed has no current aggregate
    /// or that aggregate is stale. Returns `None` if the feed's history is
    /// empty or unavailable, or if `records` is zero.
    pub fn prices(
        env: Env,
        asset: ReflectorAsset,
        records: u32,
    ) -> Option<Vec<ReflectorPriceData>> {
        let rounds = history_rounds(&env, &asset, u64::MAX, records)?;
        if rounds.is_empty() {
            return None;
        }
        Some(rounds)
    }
}

/// Returns up to `limit` history rounds of `asset`, newest first, as a
/// Reflector-style answer. Uses only the stored history entries written at or
/// before `written_by` (seconds).
///
/// A round is the newest such entry of one resolution bucket at the live
/// resolution. It carries the entry's price, stamped with the bucket start in
/// seconds ([`bucket_start_seconds`] of the entry's write timestamp). Round
/// timestamps are distinct, at least one resolution apart, and never later than
/// the ledger time. The function skips an entry whose bucket is not older than
/// the round before it.
///
/// Returns `None` if `asset` has no feed mapping or `read_price_history` fails
/// for its feed.
fn history_rounds(
    env: &Env,
    asset: &ReflectorAsset,
    written_by: u64,
    limit: u32,
) -> Option<Vec<ReflectorPriceData>> {
    let feed_id = load_feed_id(env, asset)?;
    let history = XoxnoOracle::read_price_history(env.clone(), feed_id, MAX_HISTORY_LEN).ok()?;
    let resolution = load_resolution(env);

    let mut rounds = Vec::new(env);
    let mut newer_round: Option<u64> = None;
    for entry in history.iter() {
        if rounds.len() >= limit {
            break;
        }
        if millis_to_seconds(entry.write_timestamp) > written_by {
            continue;
        }
        let timestamp = bucket_start_seconds(entry.write_timestamp, resolution);
        if newer_round.is_some_and(|newer| timestamp >= newer) {
            continue;
        }
        newer_round = Some(timestamp);
        rounds.push_back(ReflectorPriceData {
            price: u256_to_i128(env, &entry.price),
            timestamp,
        });
    }
    Some(rounds)
}

/// Converts a `RedStonePriceData` entry into `ReflectorPriceData`. Narrows the
/// U256 price to i128 and scales the millisecond package timestamp down to
/// seconds.
fn to_reflector_price_data(env: &Env, data: &RedStonePriceData) -> ReflectorPriceData {
    let price = u256_to_i128(env, &data.price);
    let timestamp = millis_to_seconds(data.package_timestamp);
    ReflectorPriceData { price, timestamp }
}
