//! Persistent-storage operations for the asset/feed registry: mapping
//! between `ReflectorAsset`s and feed ids, listing all registered assets
//! and feeds, tracking which feeds a signer submits for, clearing a feed's
//! aggregate/history/submission state, and the signer/feed authorization
//! checks used by the rest of the contract.

use common::oracle::providers::reflector::ReflectorAsset;
use soroban_sdk::{Address, Env, String, Vec};

use crate::storage::config::load_signers;
use crate::storage::index::{
    asset_count, feed_count, feed_index_contains, feed_index_insert, feed_index_remove,
};
use crate::storage::prices::{remove_aggregate, remove_history, remove_submission};
use crate::storage::ttl::renew_persistent_key;
use crate::storage::DataKey;
use crate::Error;

/// Loads the feed id mapped to `asset`. Renews the mapping's TTL if it exists.
pub(crate) fn load_feed_id(env: &Env, asset: &ReflectorAsset) -> Option<String> {
    let key = DataKey::FeedMapping(asset.clone());
    env.storage().persistent().get(&key).inspect(|_| {
        renew_persistent_key(env, &key);
    })
}

/// Loads the `ReflectorAsset` that owns `feed_id`. Renews the mapping's TTL if it exists.
pub(crate) fn load_feed_owner(env: &Env, feed_id: &String) -> Option<ReflectorAsset> {
    let key = DataKey::FeedOwner(feed_id.clone());
    env.storage().persistent().get(&key).inspect(|_| {
        renew_persistent_key(env, &key);
    })
}

/// Returns true if `asset` has a stored feed mapping. Does not renew any TTL.
pub(crate) fn has_feed_mapping(env: &Env, asset: &ReflectorAsset) -> bool {
    env.storage()
        .persistent()
        .has(&DataKey::FeedMapping(asset.clone()))
}

/// Stores the bidirectional mapping between `asset` and `feed_id`, renewing the
/// TTL of both directions.
pub(crate) fn map_feed(env: &Env, asset: &ReflectorAsset, feed_id: &String) {
    let mapping_key = DataKey::FeedMapping(asset.clone());
    env.storage().persistent().set(&mapping_key, feed_id);
    renew_persistent_key(env, &mapping_key);

    let owner_key = DataKey::FeedOwner(feed_id.clone());
    env.storage().persistent().set(&owner_key, asset);
    renew_persistent_key(env, &owner_key);
}

/// Removes `asset`'s feed mapping and returns the feed id it mapped to, without
/// renewing any TTL. Returns `None` and removes nothing if `asset` has no mapping.
pub(crate) fn take_feed_mapping(env: &Env, asset: &ReflectorAsset) -> Option<String> {
    let key = DataKey::FeedMapping(asset.clone());
    let feed_id: Option<String> = env.storage().persistent().get(&key);
    if feed_id.is_some() {
        env.storage().persistent().remove(&key);
    }
    feed_id
}

/// Removes `asset`'s feed mapping, if any, discarding the mapped feed id.
pub(crate) fn remove_feed_mapping(env: &Env, asset: &ReflectorAsset) {
    env.storage()
        .persistent()
        .remove(&DataKey::FeedMapping(asset.clone()));
}

/// Returns every registered asset, reading each from its indexed slot and renewing its TTL.
/// Skips any slot without a stored value.
pub(crate) fn load_all_assets(env: &Env) -> Vec<ReflectorAsset> {
    let count = asset_count(env);
    let mut out = Vec::new(env);
    for i in 0..count {
        let key = DataKey::AssetAt(i);
        if let Some(asset) = env.storage().persistent().get(&key) {
            renew_persistent_key(env, &key);
            out.push_back(asset);
        }
    }
    out
}

/// Returns every registered feed id, reading each from its indexed slot and renewing its TTL.
/// Skips any slot without a stored value.
pub(crate) fn load_all_feeds(env: &Env) -> Vec<String> {
    let count = feed_count(env);
    let mut out = Vec::new(env);
    for i in 0..count {
        let key = DataKey::FeedAt(i);
        if let Some(feed_id) = env.storage().persistent().get::<DataKey, String>(&key) {
            renew_persistent_key(env, &key);
            out.push_back(feed_id);
        }
    }
    out
}

/// Ensures `feed_id` is present in the feed index: renews its TTL if already indexed, otherwise
/// inserts it.
pub(crate) fn ensure_known_feed(env: &Env, feed_id: &String) {
    if !renew_known_feed(env, feed_id) {
        feed_index_insert(env, feed_id.clone());
    }
}

/// Renews the TTL of `feed_id`'s index entry and its indexed slot if `feed_id` is already
/// indexed. Returns whether it was indexed.
pub(crate) fn renew_known_feed(env: &Env, feed_id: &String) -> bool {
    let index_key = DataKey::FeedIndex(feed_id.clone());
    match env.storage().persistent().get::<DataKey, u32>(&index_key) {
        Some(slot) => {
            renew_persistent_key(env, &index_key);
            renew_persistent_key(env, &DataKey::FeedAt(slot));
            true
        }
        None => false,
    }
}

/// Adds `feed_id` to `signer`'s list of feeds if not already present, then renews the list's
/// TTL. If `feed_id` is already present, only renews the TTL.
pub(crate) fn record_signer_feed(env: &Env, signer: &Address, feed_id: &String) {
    let key = DataKey::SignerFeeds(signer.clone());
    let mut feeds: Vec<String> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env));
    if feeds.contains(feed_id) {
        renew_persistent_key(env, &key);
        return;
    }
    feeds.push_back(feed_id.clone());
    env.storage().persistent().set(&key, &feeds);
    renew_persistent_key(env, &key);
}

/// Loads the list of feed ids `signer` submits for. Returns an empty vector if none are recorded.
pub(crate) fn load_signer_feeds(env: &Env, signer: &Address) -> Vec<String> {
    env.storage()
        .persistent()
        .get(&DataKey::SignerFeeds(signer.clone()))
        .unwrap_or_else(|| Vec::new(env))
}

/// Removes `signer`'s entire recorded feed list, if any.
pub(crate) fn clear_signer_feeds(env: &Env, signer: &Address) {
    env.storage()
        .persistent()
        .remove(&DataKey::SignerFeeds(signer.clone()));
}

/// Removes `feed_id` from `signer`'s list of feeds. If `signer` has no recorded feeds, this is a
/// no-op. If removing `feed_id` leaves the list empty, deletes the list entry entirely; otherwise
/// writes the reduced list back and renews its TTL.
pub(crate) fn remove_signer_feed(env: &Env, signer: &Address, feed_id: &String) {
    let key = DataKey::SignerFeeds(signer.clone());
    let Some(feeds): Option<Vec<String>> = env.storage().persistent().get(&key) else {
        return;
    };
    let mut kept = Vec::new(env);
    for f in feeds.iter() {
        if &f != feed_id {
            kept.push_back(f);
        }
    }
    if kept.is_empty() {
        env.storage().persistent().remove(&key);
    } else {
        env.storage().persistent().set(&key, &kept);
        renew_persistent_key(env, &key);
    }
}

/// Removes all state associated with `feed_id`: its current aggregate, its price history, every
/// signer's latest submission for it, its entry in each submitting signer's feed list, its
/// owning-asset mapping, and its entry in the feed index.
pub(crate) fn clear_feed_state(env: &Env, feed_id: &String) {
    remove_aggregate(env, feed_id);
    remove_history(env, feed_id);
    for signer in load_signers(env).iter() {
        remove_submission(env, feed_id, &signer);
        remove_signer_feed(env, &signer, feed_id);
    }
    env.storage()
        .persistent()
        .remove(&DataKey::FeedOwner(feed_id.clone()));
    feed_index_remove(env, feed_id);
}

/// Returns `Error::NotAuthorizedSigner` if `signer` is not in the configured signer set.
pub(crate) fn require_registered_signer(env: &Env, signer: &Address) -> Result<(), Error> {
    let signers = load_signers(env);
    if !signers.contains(signer) {
        return Err(Error::NotAuthorizedSigner);
    }
    Ok(())
}

/// Returns `Error::FeedNotKnown` if `feed_id` is not present in the feed index.
pub(crate) fn require_known_feed(env: &Env, feed_id: &String) -> Result<(), Error> {
    if !feed_index_contains(env, feed_id) {
        return Err(Error::FeedNotKnown);
    }
    Ok(())
}
