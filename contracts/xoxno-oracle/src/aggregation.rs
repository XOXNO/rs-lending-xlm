//! Write-path guards, per-signer submission storage, and write-time median.
//!
//! Absolute age filter, then relative cluster against the freshest peer. Below
//! threshold: clear aggregate and history (raw submissions stay). Aggregation
//! at write keeps reads O(1) in signer count.

use common::constants::MS_PER_SECOND;
use common::oracle::observation::{MAX_FUTURE_SKEW_SECONDS, MAX_TWAP_RECORDS};
use common::oracle::providers::redstone::RedStonePriceData;

use soroban_sdk::{Address, Env, String, Vec, U256};

use crate::storage::{
    load_max_relative_skew, load_max_submission_age, load_resolution, load_signers, load_threshold,
    record_signer_feed, renew_known_feed, renew_persistent_key, DataKey, SignerSubmission,
};
use crate::Error;

/// Bounded FIFO cap on `History(feed_id)`. Matches the shared TWAP record cap
/// so retained history depth and the caller's TWAP window stay aligned.
pub(crate) const MAX_HISTORY_LEN: u32 = MAX_TWAP_RECORDS;

/// Submit-time ceiling on any single signer price (`1e24`).
pub(crate) const MAX_SUBMITTED_PRICE: i128 = 1_000_000_000_000_000_000_000_000;

/// Rejects package timestamps more than `MAX_FUTURE_SKEW_SECONDS` ahead of ledger time.
pub(crate) fn require_not_future(env: &Env, package_timestamp: u64) -> Result<(), Error> {
    let ts_secs = package_timestamp / MS_PER_SECOND;
    let max_future = env
        .ledger()
        .timestamp()
        .saturating_add(MAX_FUTURE_SKEW_SECONDS);
    if ts_secs > max_future {
        return Err(Error::FutureTimestamp);
    }
    Ok(())
}

/// Rejects package timestamps older than `MaxSubmissionAgeSeconds`.
pub(crate) fn require_fresh_submission(env: &Env, package_timestamp: u64) -> Result<(), Error> {
    let ts_secs = package_timestamp / MS_PER_SECOND;
    let now = env.ledger().timestamp();
    if now.saturating_sub(ts_secs) > load_max_submission_age(env) {
        return Err(Error::StaleSubmission);
    }
    Ok(())
}

/// Rejects a package timestamp older than this signer's stored observation so
/// a signer cannot re-pin observation time by overwriting a fresher value.
pub(crate) fn require_monotonic_package(
    env: &Env,
    feed_id: &String,
    signer: &Address,
    package_timestamp: u64,
) -> Result<(), Error> {
    let key = DataKey::LatestSubmission(feed_id.clone(), signer.clone());
    if let Some(prev) = env
        .storage()
        .persistent()
        .get::<DataKey, SignerSubmission>(&key)
    {
        if package_timestamp < prev.package_timestamp {
            return Err(Error::StaleSubmission);
        }
    }
    Ok(())
}

/// Persists the signer's latest observation and renews allowlist TTL for the feed.
pub(crate) fn store_submission(
    env: &Env,
    feed_id: &String,
    signer: &Address,
    price: i128,
    package_timestamp: u64,
) {
    record_signer_feed(env, signer, feed_id);
    // Submit renews submission/aggregate keys; renew the allowlist gate too so
    // an active feed does not lose its admission entry under TTL alone.
    renew_known_feed(env, feed_id);
    let submission = SignerSubmission {
        price,
        package_timestamp,
    };
    let key = DataKey::LatestSubmission(feed_id.clone(), signer.clone());
    env.storage().persistent().set(&key, &submission);
    renew_persistent_key(env, &key);
}

/// Rebuilds `CurrentAggregate` and `History` for `feed_id` from absolute-fresh
/// submissions that also pass the relative cluster filter. Below threshold,
/// clears aggregate and history; raw submissions remain.
pub(crate) fn recompute_aggregate(env: &Env, feed_id: &String) {
    let signers = load_signers(env);
    let max_submission_age = load_max_submission_age(env);
    let max_relative_skew = load_max_relative_skew(env);
    let now = env.ledger().timestamp();

    let mut kept_prices: Vec<i128> = Vec::new(env);
    let mut kept_ts: Vec<u64> = Vec::new(env);

    for signer in signers.iter() {
        let key = DataKey::LatestSubmission(feed_id.clone(), signer.clone());
        let Some(submission) = env
            .storage()
            .persistent()
            .get::<DataKey, SignerSubmission>(&key)
        else {
            continue;
        };

        // `package_timestamp` is milliseconds; ledger clock is seconds.
        let age_seconds = now.saturating_sub(submission.package_timestamp / MS_PER_SECOND);
        if age_seconds > max_submission_age {
            continue;
        }

        kept_prices.push_back(submission.price);
        kept_ts.push_back(submission.package_timestamp);
    }

    let threshold = load_threshold(env);
    if kept_prices.len() < threshold {
        clear_aggregate_and_history(env, feed_id);
        return;
    }

    // Drop peers too far behind the freshest absolute-fresh submission so a
    // lagging-but-in-window signer cannot pin `package_timestamp`.
    let mut newest_ts: u64 = 0;
    for i in 0..kept_ts.len() {
        let ts = kept_ts.get_unchecked(i);
        newest_ts = newest_ts.max(ts);
    }
    let skew_ms = max_relative_skew.saturating_mul(MS_PER_SECOND);

    let mut clustered_prices: Vec<i128> = Vec::new(env);
    let mut oldest_package_timestamp: u64 = u64::MAX;
    for i in 0..kept_ts.len() {
        let ts = kept_ts.get_unchecked(i);
        if newest_ts.saturating_sub(ts) > skew_ms {
            continue;
        }
        clustered_prices.push_back(kept_prices.get_unchecked(i));
        oldest_package_timestamp = oldest_package_timestamp.min(ts);
    }

    if clustered_prices.len() < threshold {
        clear_aggregate_and_history(env, feed_id);
        return;
    }

    let median = median_of(&clustered_prices);
    let write_timestamp = now * MS_PER_SECOND;
    let aggregate = RedStonePriceData {
        price: U256::from_u128(env, median as u128),
        package_timestamp: oldest_package_timestamp,
        write_timestamp,
    };

    let aggregate_key = DataKey::CurrentAggregate(feed_id.clone());
    env.storage().persistent().set(&aggregate_key, &aggregate);
    renew_persistent_key(env, &aggregate_key);
    push_history(env, feed_id, aggregate);
}

fn clear_aggregate_and_history(env: &Env, feed_id: &String) {
    // `price` / `prices` read history without re-checking quorum, so history
    // must leave with the aggregate when below threshold.
    env.storage()
        .persistent()
        .remove(&DataKey::CurrentAggregate(feed_id.clone()));
    env.storage()
        .persistent()
        .remove(&DataKey::History(feed_id.clone()));
}

/// Insertion sort on a clone. Signer counts are small (single digits).
fn sorted_copy(prices: &Vec<i128>) -> Vec<i128> {
    let mut sorted = prices.clone();
    let len = sorted.len();
    for i in 1..len {
        let key = sorted.get_unchecked(i);
        let mut j = i;
        while let Some(previous_index) = j.checked_sub(1) {
            let prev = sorted.get_unchecked(previous_index);
            if prev <= key {
                break;
            }
            sorted.set(j, prev);
            j = previous_index;
        }
        sorted.set(j, key);
    }
    sorted
}

/// Lower median at index `(len - 1) / 2`. No even-count average — an extreme
/// peer cannot half-pull the reported price.
fn median_of(prices: &Vec<i128>) -> i128 {
    let sorted = sorted_copy(prices);
    let len = sorted.len();
    // Callers gate on `len >= threshold >= 1`.
    sorted.get_unchecked((len - 1) / 2)
}

/// Appends `aggregate` to `History(feed_id)` as a `resolution`-spaced sample.
/// A sample inside the newest sample's resolution window overwrites it in
/// place so the bounded FIFO spans ~`MAX_HISTORY_LEN` buckets. `resolution == 0`
/// appends every submission.
fn push_history(env: &Env, feed_id: &String, aggregate: RedStonePriceData) {
    let key = DataKey::History(feed_id.clone());
    let mut history: Vec<RedStonePriceData> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env));

    let resolution_ms = u64::from(load_resolution(env)) * MS_PER_SECOND;
    let len = history.len();
    let replace_last = len > 0 && {
        let last = history.get_unchecked(len - 1);
        aggregate.write_timestamp < last.write_timestamp.saturating_add(resolution_ms)
    };

    if replace_last {
        history.set(len - 1, aggregate);
    } else {
        if len >= MAX_HISTORY_LEN {
            history.pop_front();
        }
        history.push_back(aggregate);
    }
    env.storage().persistent().set(&key, &history);
    renew_persistent_key(env, &key);
}

#[cfg(test)]
#[path = "../tests/unit/aggregation.rs"]
mod tests;
