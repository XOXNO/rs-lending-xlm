//! Write path: per-signer submission storage and write-time median
//! aggregation. Aggregation runs inside `submit_price`/`submit_prices` so
//! reads stay O(1) regardless of signer count.

use common::constants::MS_PER_SECOND;
use common::oracle::observation::MAX_TWAP_RECORDS;
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
/// fresh, the cached `CurrentAggregate` is removed (fail-safe: reads return
/// `NoDataForFeed` rather than a stale/poisoned price) — the signer's raw
/// submission recorded by the caller stays in place regardless.
pub(crate) fn recompute_aggregate(env: &Env, feed_id: &String) {
    let signers = load_signers(env);
    let max_stale = load_max_stale_seconds(env);
    let now = env.ledger().timestamp();

    let mut kept_prices: Vec<i128> = Vec::new(env);
    let mut max_package_timestamp: u64 = 0;

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
        if submission.package_timestamp > max_package_timestamp {
            max_package_timestamp = submission.package_timestamp;
        }
    }

    let threshold = load_threshold(env);
    if kept_prices.len() < threshold {
        // Below quorum: evict the cached aggregate so reads fail safe with
        // `NoDataForFeed` instead of serving a stale value that may include a
        // just-removed (possibly compromised) signer's price. `History` is
        // intentionally left — it is append-only TWAP history and the
        // controller re-checks TWAP staleness on those reads.
        env.storage()
            .persistent()
            .remove(&DataKey::CurrentAggregate(feed_id.clone()));
        return;
    }

    let median = median_of(&kept_prices);
    let write_timestamp = now * MS_PER_SECOND;
    let aggregate = RedStonePriceData {
        price: U256::from_u128(env, median as u128),
        package_timestamp: max_package_timestamp,
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
