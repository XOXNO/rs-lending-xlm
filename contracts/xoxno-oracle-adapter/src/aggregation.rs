//! Write path: per-signer submission storage and write-time median
//! aggregation. Aggregation runs inside `submit_price`/`submit_prices` so
//! reads stay O(1) regardless of signer count.

use common::constants::MS_PER_SECOND;
use common::oracle::observation::{MAX_FUTURE_SKEW_SECONDS, MAX_TWAP_RECORDS};
use common::oracle::providers::redstone::RedStonePriceData;
use soroban_sdk::{contractimpl, Address, Env, String, Vec, U256};

use crate::storage::{
    load_max_stale_seconds, load_signers, load_threshold, record_known_feed, record_signer_feed,
    renew_oracle_instance, renew_persistent_key, require_registered_signer, DataKey,
    SignerSubmission,
};
use crate::{Error, XoxnoOracle, XoxnoOracleArgs, XoxnoOracleClient};

/// Bounded FIFO cap on `History(feed_id)`, and the request size used when the
/// SEP-40 `price()`/`prices()` reads need "the whole retained history". Aliases
/// the shared TWAP record cap so retained history depth and the caller's TWAP
/// window stay provably in sync.
pub(crate) const MAX_HISTORY_LEN: u32 = MAX_TWAP_RECORDS;

/// Submit-time ceiling on any single signer price (1e24). Far above any
/// realistic USD-scaled price yet far below `i128::MAX` (~1.7e38), so summing
/// two accepted prices in the even-count median can never overflow.
pub(crate) const MAX_SUBMITTED_PRICE: i128 = 1_000_000_000_000_000_000_000_000;

#[contractimpl]
impl XoxnoOracle {
    // -----------------------------------------------------------------
    // Write functions — bot wallets, gated by `signer.require_auth()`.
    // -----------------------------------------------------------------

    /// Records `signer`'s latest observation for `feed_id` and recomputes
    /// the cached aggregate for that feed.
    ///
    /// # Errors
    /// * `NotAuthorizedSigner` - `signer` is not a registered signer.
    /// * `InvalidPrice` - `price <= 0`.
    /// * `PriceOutOfRange` - `price > MAX_SUBMITTED_PRICE`.
    /// * `FutureTimestamp` - `package_timestamp` is more than
    ///   `MAX_FUTURE_SKEW_SECONDS` ahead of the ledger clock.
    pub fn submit_price(
        env: Env,
        signer: Address,
        feed_id: String,
        price: i128,
        package_timestamp: u64,
    ) -> Result<(), Error> {
        renew_oracle_instance(&env);
        signer.require_auth();
        require_registered_signer(&env, &signer)?;
        if price <= 0 {
            return Err(Error::InvalidPrice);
        }
        if price > MAX_SUBMITTED_PRICE {
            return Err(Error::PriceOutOfRange);
        }
        require_not_future(&env, package_timestamp)?;

        store_submission(&env, &feed_id, &signer, price, package_timestamp);
        recompute_aggregate(&env, &feed_id);
        Ok(())
    }

    /// Records `signer`'s latest observations for multiple feeds in one
    /// call, sharing a single `package_timestamp` and one auth check.
    ///
    /// # Errors
    /// * `NotAuthorizedSigner` - `signer` is not a registered signer.
    /// * `LengthMismatch` - `feed_ids.len() != prices.len()`.
    /// * `InvalidPrice` - any `prices[i] <= 0` (checked upfront; no partial
    ///   application on failure).
    /// * `PriceOutOfRange` - any `prices[i] > MAX_SUBMITTED_PRICE` (checked
    ///   upfront; no partial application on failure).
    /// * `FutureTimestamp` - the shared `package_timestamp` is more than
    ///   `MAX_FUTURE_SKEW_SECONDS` ahead of the ledger clock.
    pub fn submit_prices(
        env: Env,
        signer: Address,
        feed_ids: Vec<String>,
        prices: Vec<i128>,
        package_timestamp: u64,
    ) -> Result<(), Error> {
        renew_oracle_instance(&env);
        signer.require_auth();
        require_registered_signer(&env, &signer)?;
        if feed_ids.len() != prices.len() {
            return Err(Error::LengthMismatch);
        }
        require_not_future(&env, package_timestamp)?;
        for price in prices.iter() {
            if price <= 0 {
                return Err(Error::InvalidPrice);
            }
            if price > MAX_SUBMITTED_PRICE {
                return Err(Error::PriceOutOfRange);
            }
        }

        for (feed_id, price) in feed_ids.iter().zip(prices.iter()) {
            store_submission(&env, &feed_id, &signer, price, package_timestamp);
            recompute_aggregate(&env, &feed_id);
        }
        Ok(())
    }
}

/// Rejects a `package_timestamp` (milliseconds) more than
/// `MAX_FUTURE_SKEW_SECONDS` ahead of the ledger clock. `recompute_aggregate`
/// treats any future timestamp as age zero, so without this a signer clock/unit
/// bug could cache a far-future `package_timestamp` that then reverts every
/// downstream read on the reader's future-timestamp guard until corrected.
fn require_not_future(env: &Env, package_timestamp: u64) -> Result<(), Error> {
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
/// signer's latest submission. Submissions older than `MaxStaleSeconds` are
/// excluded from consideration. If fewer than `Threshold` submissions remain
/// fresh, the cached `CurrentAggregate` and `History` are both removed
/// (fail-safe: spot and TWAP reads return `NoDataForFeed`/`None` rather than a
/// stale/poisoned price) — the signer's raw submission recorded by the caller
/// stays in place regardless.
pub(crate) fn recompute_aggregate(env: &Env, feed_id: &String) {
    let signers = load_signers(env);
    let max_stale = load_max_stale_seconds(env);
    let now = env.ledger().timestamp();

    let mut kept_prices: Vec<i128> = Vec::new(env);
    // Oldest contributing observation time: an aggregate is only as fresh as
    // its stalest included submission, so downstream freshness checks must see
    // that bound (using the freshest would let a near-stale median input hide
    // behind a current one).
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

        // `package_timestamp` is milliseconds (RedStone convention);
        // `ledger().timestamp()` is seconds. Divide before comparing, or
        // every submission looks 1000x staler than it is. Saturate rather
        // than subtract directly: a `package_timestamp` at or after `now`
        // (clock skew, or same-second submission) must read as "not stale".
        let age_seconds = now.saturating_sub(submission.package_timestamp / MS_PER_SECOND);
        if age_seconds > max_stale {
            continue;
        }

        kept_prices.push_back(submission.price);
        oldest_package_timestamp = oldest_package_timestamp.min(submission.package_timestamp);
    }

    let threshold = load_threshold(env);
    if kept_prices.len() < threshold {
        // Below quorum: evict both the cached aggregate and the TWAP history so
        // spot and TWAP reads fail safe (`NoDataForFeed`/`None`) instead of
        // serving values that may include a just-removed (possibly compromised)
        // signer's price. History is cleared too because `prices()`/`price()`
        // read it directly and the controller only re-checks sample
        // timestamps, not whether the samples came from the current quorum.
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

/// Insertion sort — no_std friendly, and fine for the small signer counts
/// (well under 10) this contract is designed for.
fn sorted_copy(prices: &Vec<i128>) -> Vec<i128> {
    let mut sorted = prices.clone();
    let len = sorted.len();
    for i in 1..len {
        let key = sorted.get(i).unwrap(); // safe: i in 1..len
        let mut j = i;
        while j > 0 && sorted.get(j - 1).unwrap() > key {
            // safe: j-1 < j <= len
            let prev = sorted.get(j - 1).unwrap(); // safe: j-1 < j <= len
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
        sorted.get(mid).unwrap() // safe: mid = len/2 < len for odd len >= 1
    } else {
        let a = sorted.get(mid - 1).unwrap(); // safe: even len >= 2 so mid >= 1
        let b = sorted.get(mid).unwrap(); // safe: mid = len/2 < len
        // Overflow-safe midpoint: both prices are > 0 and sorted so b >= a,
        // thus `b - a` can't overflow and `a + (b - a)/2` stays within [a, b],
        // avoiding the `a + b` overflow that `overflow-checks` would abort on.
        a + (b - a) / 2
    }
}

/// Pushes `aggregate` onto `History(feed_id)`, evicting the oldest entry
/// once the bounded FIFO would exceed `MAX_HISTORY_LEN`.
pub(crate) fn push_history(env: &Env, feed_id: &String, aggregate: RedStonePriceData) {
    let key = DataKey::History(feed_id.clone());
    let mut history: Vec<RedStonePriceData> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env));

    if history.len() >= MAX_HISTORY_LEN {
        history.pop_front();
    }
    history.push_back(aggregate);
    env.storage().persistent().set(&key, &history);
    renew_persistent_key(env, &key);
}
