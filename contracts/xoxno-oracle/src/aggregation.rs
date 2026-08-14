//! Submission validation and price aggregation for the oracle: freshness and
//! monotonicity checks on incoming submissions, and the median-based
//! aggregate computation that clusters signer submissions by timestamp skew
//! and writes the resulting price and history.

use common::constants::MS_PER_SECOND;
use common::oracle::observation::{MAX_FUTURE_SKEW_SECONDS, MAX_TWAP_RECORDS};
use common::oracle::providers::redstone::RedStonePriceData;

use soroban_sdk::{Address, Env, String, Vec, U256};

use crate::storage::{
    load_history, load_max_relative_skew, load_max_submission_age, load_resolution, load_signers,
    load_submission, load_threshold, record_signer_feed, remove_aggregate, renew_known_feed,
    store_aggregate, store_history, store_submission_record,
};
use crate::Error;

/// Maximum number of aggregate entries retained in a feed's price history.
pub(crate) const MAX_HISTORY_LEN: u32 = MAX_TWAP_RECORDS;

/// Upper bound on a submitted price value.
pub(crate) const MAX_SUBMITTED_PRICE: i128 = 1_000_000_000_000_000_000_000_000;

/// Rejects `package_timestamp` (milliseconds) if it lies further in the future
/// than `MAX_FUTURE_SKEW_SECONDS` past the current ledger time (seconds).
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

/// Rejects `package_timestamp` (milliseconds) if its age in seconds relative to
/// the current ledger time exceeds the configured maximum submission age
/// (seconds).
pub(crate) fn require_fresh_submission(env: &Env, package_timestamp: u64) -> Result<(), Error> {
    let ts_secs = package_timestamp / MS_PER_SECOND;
    let now = env.ledger().timestamp();
    if now.saturating_sub(ts_secs) > load_max_submission_age(env) {
        return Err(Error::StaleSubmission);
    }
    Ok(())
}

/// Rejects `package_timestamp` (milliseconds) if `signer` has a stored
/// submission for `feed_id` with a later package timestamp. Passes if no prior
/// submission exists.
pub(crate) fn require_monotonic_package(
    env: &Env,
    feed_id: &String,
    signer: &Address,
    package_timestamp: u64,
) -> Result<(), Error> {
    if let Some(prev) = load_submission(env, feed_id, signer) {
        if package_timestamp < prev.package_timestamp {
            return Err(Error::StaleSubmission);
        }
    }
    Ok(())
}

/// Records the signer's feed membership, marks the feed as recently touched,
/// and overwrites the signer's latest submission for `feed_id` with `price`
/// and `package_timestamp` (milliseconds).
pub(crate) fn store_submission(
    env: &Env,
    feed_id: &String,
    signer: &Address,
    price: i128,
    package_timestamp: u64,
) {
    record_signer_feed(env, signer, feed_id);

    renew_known_feed(env, feed_id);
    store_submission_record(env, feed_id, signer, price, package_timestamp);
}

/// Recomputes the current aggregate price for `feed_id` from all signers'
/// latest submissions. Discards submissions older than the maximum
/// submission age, then keeps only those within `max_relative_skew` of the
/// newest surviving timestamp. If fewer than `threshold` submissions survive
/// either filter, clears the feed's aggregate and history. Otherwise takes
/// the median of the clustered prices, writes it as the new aggregate with
/// the oldest clustered timestamp as its package timestamp, and appends it
/// to the feed's history.
pub(crate) fn recompute_aggregate(env: &Env, feed_id: &String) {
    let signers = load_signers(env);
    let max_submission_age = load_max_submission_age(env);
    let max_relative_skew = load_max_relative_skew(env);
    let now = env.ledger().timestamp();

    let mut kept_prices: Vec<i128> = Vec::new(env);
    let mut kept_ts: Vec<u64> = Vec::new(env);

    for signer in signers.iter() {
        let Some(submission) = load_submission(env, feed_id, &signer) else {
            continue;
        };

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

    store_aggregate(env, feed_id, &aggregate);
    push_history(env, feed_id, aggregate);
}

/// Removes the current aggregate and history entries for `feed_id`.
fn clear_aggregate_and_history(env: &Env, feed_id: &String) {
    // Only the live aggregate is cleared: a transient quorum miss must not
    // destroy accumulated history that consumers age off themselves.
    remove_aggregate(env, feed_id);
}

/// Returns an ascending-sorted copy of `prices`, computed with an in-place
/// insertion sort.
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

/// Returns the lower median of `prices`. Panics if `prices` is empty.
fn median_of(prices: &Vec<i128>) -> i128 {
    let sorted = sorted_copy(prices);
    let len = sorted.len();

    sorted.get_unchecked((len - 1) / 2)
}

/// Appends `aggregate` to `feed_id`'s history, or overwrites the last entry
/// if its write timestamp falls within one resolution period of the previous
/// entry. Evicts the oldest entry when the history reaches `MAX_HISTORY_LEN`.
fn push_history(env: &Env, feed_id: &String, aggregate: RedStonePriceData) {
    let mut history: Vec<RedStonePriceData> =
        load_history(env, feed_id).unwrap_or_else(|| Vec::new(env));

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
    store_history(env, feed_id, &history);
}

#[cfg(test)]
#[path = "../tests/unit/aggregation.rs"]
mod tests;
