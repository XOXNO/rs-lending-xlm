//! Persistent-storage operations for price data: per-signer latest
//! submissions, the current aggregate per feed, and the bounded price
//! history per feed. Owns the `LatestSubmission`, `CurrentAggregate`, and
//! `History` key families so callers never construct storage keys or choose
//! TTL policy themselves.

use common::oracle::providers::redstone::RedStonePriceData;
use soroban_sdk::{Address, Env, String, Vec};

use crate::storage::ttl::renew_persistent_key;
use crate::storage::{DataKey, SignerSubmission};

/// Loads `signer`'s latest stored submission for `feed_id` without renewing
/// its TTL. Returns `None` if no submission is stored.
pub(crate) fn load_submission(
    env: &Env,
    feed_id: &String,
    signer: &Address,
) -> Option<SignerSubmission> {
    env.storage()
        .persistent()
        .get(&DataKey::LatestSubmission(feed_id.clone(), signer.clone()))
}

/// Overwrites `signer`'s latest submission for `feed_id` with `price` and
/// `package_timestamp` (milliseconds), then renews the entry's TTL.
pub(crate) fn store_submission_record(
    env: &Env,
    feed_id: &String,
    signer: &Address,
    price: i128,
    package_timestamp: u64,
) {
    let submission = SignerSubmission {
        price,
        package_timestamp,
    };
    let key = DataKey::LatestSubmission(feed_id.clone(), signer.clone());
    env.storage().persistent().set(&key, &submission);
    renew_persistent_key(env, &key);
}

/// Removes `signer`'s latest stored submission for `feed_id`, if any.
pub(crate) fn remove_submission(env: &Env, feed_id: &String, signer: &Address) {
    env.storage()
        .persistent()
        .remove(&DataKey::LatestSubmission(feed_id.clone(), signer.clone()));
}

/// Loads the current aggregate for `feed_id`, renewing its TTL when present.
/// Returns `None` if no aggregate is stored.
pub(crate) fn load_aggregate(env: &Env, feed_id: &String) -> Option<RedStonePriceData> {
    let key = DataKey::CurrentAggregate(feed_id.clone());
    env.storage().persistent().get(&key).inspect(|_| {
        renew_persistent_key(env, &key);
    })
}

/// Overwrites the current aggregate for `feed_id` and renews its TTL.
pub(crate) fn store_aggregate(env: &Env, feed_id: &String, aggregate: &RedStonePriceData) {
    let key = DataKey::CurrentAggregate(feed_id.clone());
    env.storage().persistent().set(&key, aggregate);
    renew_persistent_key(env, &key);
}

/// Removes the current aggregate for `feed_id`, if any.
pub(crate) fn remove_aggregate(env: &Env, feed_id: &String) {
    env.storage()
        .persistent()
        .remove(&DataKey::CurrentAggregate(feed_id.clone()));
}

/// Loads the price history for `feed_id` without renewing its TTL. Returns
/// `None` if no history is stored.
pub(crate) fn load_history(env: &Env, feed_id: &String) -> Option<Vec<RedStonePriceData>> {
    env.storage()
        .persistent()
        .get(&DataKey::History(feed_id.clone()))
}

/// Renews the TTL of `feed_id`'s stored price history.
pub(crate) fn renew_history(env: &Env, feed_id: &String) {
    renew_persistent_key(env, &DataKey::History(feed_id.clone()));
}

/// Overwrites the price history for `feed_id` and renews its TTL.
pub(crate) fn store_history(env: &Env, feed_id: &String, history: &Vec<RedStonePriceData>) {
    let key = DataKey::History(feed_id.clone());
    env.storage().persistent().set(&key, history);
    renew_persistent_key(env, &key);
}

/// Removes the price history for `feed_id`, if any.
pub(crate) fn remove_history(env: &Env, feed_id: &String) {
    env.storage()
        .persistent()
        .remove(&DataKey::History(feed_id.clone()));
}
