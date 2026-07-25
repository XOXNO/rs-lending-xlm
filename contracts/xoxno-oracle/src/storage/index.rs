//! Dense swap-remove indexes behind the registry. Add/remove costs O(1)
//! persistent writes instead of rewriting a growing instance blob.

use common::oracle::providers::reflector::ReflectorAsset;
use soroban_sdk::{Env, String};

use crate::storage::{ttl::renew_persistent_key, DataKey};

/// Number of occupied asset slots; also the next free index.
pub(crate) fn asset_count(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::AssetCount)
        .unwrap_or(0)
}

/// Number of occupied feed slots; also the next free index.
pub(crate) fn feed_count(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::FeedCount)
        .unwrap_or(0)
}

/// True when `feed_id` is on the known-feed allowlist.
pub(crate) fn feed_index_contains(env: &Env, feed_id: &String) -> bool {
    env.storage()
        .persistent()
        .has(&DataKey::FeedIndex(feed_id.clone()))
}

/// Appends an asset to the dense index. Caller must check it is absent first.
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

/// Swap-removes an asset: the tail slot moves into the hole and the count shrinks,
/// so the index stays dense and `load_all_assets` needs no tombstone handling.
pub(crate) fn asset_index_remove(env: &Env, asset: &ReflectorAsset) {
    let index_key = DataKey::AssetIndex(asset.clone());
    let Some(removed_at): Option<u32> = env.storage().persistent().get(&index_key) else {
        return;
    };
    env.storage().persistent().remove(&index_key);

    let count = asset_count(env);
    let last_at = count - 1;
    if removed_at != last_at {
        let last_key = DataKey::AssetAt(last_at);
        // Defensive: if the last slot archived, shrink without swap rather than panic.
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

/// Appends a feed to the dense index. Caller must check it is absent first.
pub(crate) fn feed_index_insert(env: &Env, feed_id: String) {
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

/// Swap-removes a feed from the dense index; mirrors [`asset_index_remove`].
pub(crate) fn feed_index_remove(env: &Env, feed_id: &String) {
    let index_key = DataKey::FeedIndex(feed_id.clone());
    let Some(removed_at): Option<u32> = env.storage().persistent().get(&index_key) else {
        return;
    };
    env.storage().persistent().remove(&index_key);

    let count = feed_count(env);
    let last_at = count - 1;
    if removed_at != last_at {
        let last_key = DataKey::FeedAt(last_at);
        // Defensive: if the last slot archived, shrink without swap rather than panic.
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
