//! Write-path helpers: submission guards, per-signer submission storage, and
//! write-time median aggregation. Running aggregation at write time keeps
//! reads O(1) regardless of signer count.

use common::constants::MS_PER_SECOND;
use common::oracle::observation::{MAX_FUTURE_SKEW_SECONDS, MAX_TWAP_RECORDS};
use common::oracle::providers::redstone::RedStonePriceData;

use soroban_sdk::{Address, Env, String, Vec, U256};

use crate::storage::{
    load_max_submission_age, load_resolution, load_signers, load_threshold, record_known_feed,
    record_signer_feed, renew_persistent_key, DataKey, SignerSubmission,
};
use crate::Error;

/// Bounded FIFO cap on `History(feed_id)`. Aliases the shared TWAP record cap
/// so retained history depth and the caller's TWAP window stay in sync.
pub(crate) const MAX_HISTORY_LEN: u32 = MAX_TWAP_RECORDS;

/// Submit-time ceiling on any single signer price (1e24): far above any
/// realistic USD-scaled price, far below `i128::MAX`, so the even-count
/// median's `a + (b - a) / 2` can never overflow.
pub(crate) const MAX_SUBMITTED_PRICE: i128 = 1_000_000_000_000_000_000_000_000;

/// Rejects a `package_timestamp` (milliseconds) more than
/// `MAX_FUTURE_SKEW_SECONDS` ahead of the ledger clock, so a signer clock/unit
/// bug can't cache a far-future timestamp that reverts every downstream read.
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

/// Rejects a `package_timestamp` (milliseconds) already older than the
/// `MaxSubmissionAgeSeconds` inclusion window: surfaces a units bug as an
/// explicit error and blocks backdated submissions.
pub(crate) fn require_fresh_submission(env: &Env, package_timestamp: u64) -> Result<(), Error> {
    let ts_secs = package_timestamp / MS_PER_SECOND;
    let now = env.ledger().timestamp();
    if now.saturating_sub(ts_secs) > load_max_submission_age(env) {
        return Err(Error::StaleSubmission);
    }
    Ok(())
}

pub(crate) fn store_submission(
    env: &Env,
    feed_id: &String,
    signer: &Address,
    price: i128,
    package_timestamp: u64,
) {
    record_known_feed(env, feed_id);
    record_signer_feed(env, signer, feed_id);
    let submission = SignerSubmission {
        price,
        package_timestamp,
    };
    let key = DataKey::LatestSubmission(feed_id.clone(), signer.clone());
    env.storage().persistent().set(&key, &submission);
    renew_persistent_key(env, &key);
}

/// Recomputes and caches the aggregate for `feed_id` from every registered
/// signer's latest submission. Submissions older than the
/// `MaxSubmissionAgeSeconds` window are excluded from both the median and the
/// reported observation time, so a lagging/offline signer can neither skew
/// the price nor pin the feed's freshness. Below `Threshold` fresh
/// submissions, the cached aggregate and history are removed (fail-safe:
/// reads return `NoDataForFeed`/`None`); raw submissions stay in place.
pub(crate) fn recompute_aggregate(env: &Env, feed_id: &String) {
    let signers = load_signers(env);
    let max_submission_age = load_max_submission_age(env);
    let now = env.ledger().timestamp();

    let mut kept_prices: Vec<i128> = Vec::new(env);
    // An aggregate is only as fresh as its stalest included submission, so
    // report the oldest contributing observation time.
    let mut oldest_package_timestamp: u64 = u64::MAX;

    for signer in signers.iter() {
        let key = DataKey::LatestSubmission(feed_id.clone(), signer.clone());
        let Some(submission) = env
            .storage()
            .persistent()
            .get::<DataKey, SignerSubmission>(&key)
        else {
            continue;
        };

        // `package_timestamp` is milliseconds, the ledger clock is seconds;
        // saturate so a timestamp at/after `now` reads as fresh.
        let age_seconds = now.saturating_sub(submission.package_timestamp / MS_PER_SECOND);
        if age_seconds > max_submission_age {
            continue;
        }

        kept_prices.push_back(submission.price);
        oldest_package_timestamp = oldest_package_timestamp.min(submission.package_timestamp);
    }

    let threshold = load_threshold(env);
    if kept_prices.len() < threshold {
        // Below quorum: evict aggregate AND history — `price()`/`prices()`
        // read history directly and never re-check quorum.
        env.storage()
            .persistent()
            .remove(&DataKey::CurrentAggregate(feed_id.clone()));
        env.storage()
            .persistent()
            .remove(&DataKey::History(feed_id.clone()));
        return;
    }

    let median = median_of(&kept_prices);
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

/// Insertion sort — no_std friendly, fine for the small signer counts
/// (well under 10) this contract is designed for.
fn sorted_copy(prices: &Vec<i128>) -> Vec<i128> {
    let mut sorted = prices.clone();
    let len = sorted.len();
    for i in 1..len {
        let key = sorted
            .get(i)
            .expect("invariant: i in 1..len after clone of input vec");
        let mut j = i;
        while j > 0 && sorted.get(j - 1).expect("invariant: j-1 < j <= len") > key {
            let prev = sorted.get(j - 1).expect("invariant: j-1 < j <= len");
            sorted.set(j, prev);
            j -= 1;
        }
        sorted.set(j, key);
    }
    sorted
}

fn median_of(prices: &Vec<i128>) -> i128 {
    let sorted = sorted_copy(prices);
    let len = sorted.len();
    let mid = len / 2;
    if len % 2 == 1 {
        sorted
            .get(mid)
            .expect("invariant: mid = len/2 < len for odd len >= 1")
    } else {
        let a = sorted
            .get(mid - 1)
            .expect("invariant: even len >= 2 so mid-1 valid");
        let b = sorted.get(mid).expect("invariant: mid = len/2 < len");
        // Overflow-safe midpoint: sorted so b >= a, both > 0.
        a + (b - a) / 2
    }
}

/// Records `aggregate` in `History(feed_id)` as a `resolution`-spaced sample:
/// a sample landing inside the newest sample's `resolution` window overwrites
/// it in place, so the bounded FIFO spans ~`MAX_HISTORY_LEN` buckets instead
/// of collapsing to the raw submission cadence. `resolution == 0` (unset)
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
        let last = history.get(len - 1).expect("invariant: len > 0 checked");
        aggregate.write_timestamp < last.write_timestamp.saturating_add(resolution_ms)
    };

    if replace_last {
        history.set(len - 1, aggregate); // safe: len > 0
    } else {
        if len >= MAX_HISTORY_LEN {
            history.pop_front();
        }
        history.push_back(aggregate);
    }
    env.storage().persistent().set(&key, &history);
    renew_persistent_key(env, &key);
}
