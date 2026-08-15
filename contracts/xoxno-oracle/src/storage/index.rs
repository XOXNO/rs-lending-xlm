//! Maintains two swap-remove indexed collections in persistent storage: the
//! set of tracked `ReflectorAsset`s and the set of tracked feed ids. Each
//! collection stores its elements at contiguous `*At` slots, a reverse
//! `*Index` lookup from element to slot, and a `*Count` of populated slots.
//! Removal swaps the last slot into the removed slot's position to keep
//! slots contiguous, and refreshes TTLs on every write.

use common::oracle::providers::reflector::ReflectorAsset;
use soroban_sdk::{Env, String};

use crate::storage::{ttl::renew_persistent_key, DataKey};

/// Returns the number of populated asset slots. Returns 0 if the counter is unset.
pub(in crate::storage) fn asset_count(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::AssetCount)
        .unwrap_or(0)
}

/// Returns the number of populated feed slots. Returns 0 if the counter is unset.
pub(in crate::storage) fn feed_count(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::FeedCount)
        .unwrap_or(0)
}

/// Returns true if `feed_id` has an entry in the feed index.
pub(crate) fn feed_index_contains(env: &Env, feed_id: &String) -> bool {
    env.storage()
        .persistent()
        .has(&DataKey::FeedIndex(feed_id.clone()))
}

/// Appends `asset` to the asset collection: stores it at the next free slot, records its slot in
/// the reverse index, and increments the asset count. Renews the TTL of every key it writes.
pub(crate) fn asset_index_insert(env: &Env, asset: ReflectorAsset) {
    let count = asset_count(env);
    let at_key = DataKey::AssetAt(count);
    let index_key = DataKey::AssetIndex(asset.clone());

    env.storage().persistent().set(&at_key, &asset);
    renew_persistent_key(env, &at_key);
    env.storage().persistent().set(&index_key, &count);
    renew_persistent_key(env, &index_key);

    let count_key = DataKey::AssetCount;
    env.storage().persistent().set(&count_key, &(count + 1));
    renew_persistent_key(env, &count_key);
}

/// Removes `asset` from the asset collection, or does nothing if it is not indexed. Otherwise
/// moves the asset from the last occupied slot into the removed slot to keep slots contiguous,
/// updates the moved asset's reverse-index entry, clears the vacated last slot, decrements the
/// asset count, and renews the TTL of every key it writes. Panics if the asset count is 0 while
/// the asset is still indexed.
pub(crate) fn asset_index_remove(env: &Env, asset: &ReflectorAsset) {
    let index_key = DataKey::AssetIndex(asset.clone());
    let Some(removed_at): Option<u32> = env.storage().persistent().get(&index_key) else {
        return;
    };
    env.storage().persistent().remove(&index_key);

    let count = asset_count(env);
    let last_at = count
        .checked_sub(1)
        .expect("asset index count must cover indexed asset");
    if removed_at != last_at {
        let last_key = DataKey::AssetAt(last_at);

        if let Some(moved) = env
            .storage()
            .persistent()
            .get::<DataKey, ReflectorAsset>(&last_key)
        {
            let moved_at_key = DataKey::AssetAt(removed_at);
            env.storage().persistent().set(&moved_at_key, &moved);
            renew_persistent_key(env, &moved_at_key);

            let moved_index_key = DataKey::AssetIndex(moved);
            env.storage()
                .persistent()
                .set(&moved_index_key, &removed_at);
            renew_persistent_key(env, &moved_index_key);
        }
    }
    env.storage()
        .persistent()
        .remove(&DataKey::AssetAt(last_at));

    let count_key = DataKey::AssetCount;
    env.storage().persistent().set(&count_key, &last_at);
    renew_persistent_key(env, &count_key);
}

/// Appends `feed_id` to the feed collection: stores it at the next free slot, records its slot
/// in the reverse index, and increments the feed count. Renews the TTL of every key it writes.
pub(in crate::storage) fn feed_index_insert(env: &Env, feed_id: String) {
    let count = feed_count(env);
    let at_key = DataKey::FeedAt(count);
    let index_key = DataKey::FeedIndex(feed_id.clone());

    env.storage().persistent().set(&at_key, &feed_id);
    renew_persistent_key(env, &at_key);
    env.storage().persistent().set(&index_key, &count);
    renew_persistent_key(env, &index_key);

    let count_key = DataKey::FeedCount;
    env.storage().persistent().set(&count_key, &(count + 1));
    renew_persistent_key(env, &count_key);
}

/// Removes `feed_id` from the feed collection, or does nothing if it is not indexed. Otherwise
/// moves the feed id from the last occupied slot into the removed slot to keep slots contiguous,
/// updates the moved feed id's reverse-index entry, clears the vacated last slot, decrements the
/// feed count, and renews the TTL of every key it writes. Panics if the feed count is 0 while the
/// feed id is still indexed.
pub(crate) fn feed_index_remove(env: &Env, feed_id: &String) {
    let index_key = DataKey::FeedIndex(feed_id.clone());
    let Some(removed_at): Option<u32> = env.storage().persistent().get(&index_key) else {
        return;
    };
    env.storage().persistent().remove(&index_key);

    let count = feed_count(env);
    let last_at = count
        .checked_sub(1)
        .expect("feed index count must cover indexed feed");
    if removed_at != last_at {
        let last_key = DataKey::FeedAt(last_at);

        if let Some(moved) = env.storage().persistent().get::<DataKey, String>(&last_key) {
            let moved_at_key = DataKey::FeedAt(removed_at);
            env.storage().persistent().set(&moved_at_key, &moved);
            renew_persistent_key(env, &moved_at_key);

            let moved_index_key = DataKey::FeedIndex(moved);
            env.storage()
                .persistent()
                .set(&moved_index_key, &removed_at);
            renew_persistent_key(env, &moved_index_key);
        }
    }
    env.storage().persistent().remove(&DataKey::FeedAt(last_at));

    let count_key = DataKey::FeedCount;
    env.storage().persistent().set(&count_key, &last_at);
    renew_persistent_key(env, &count_key);
}
