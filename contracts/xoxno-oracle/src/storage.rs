//! Storage keys, types, and raw storage helpers. No entrypoints live here.
//!
//! Assets and known feed ids are each kept as a swap-remove indexed set
//! (count + slot-array + reverse lookup) in persistent storage, so add/remove
//! cost O(1) persistent writes instead of rewriting a growing instance blob.

use common::constants::{
    TTL_BUMP_INSTANCE, TTL_BUMP_SHARED, TTL_THRESHOLD_INSTANCE, TTL_THRESHOLD_SHARED,
};
use common::oracle::providers::reflector::ReflectorAsset;

use soroban_sdk::{contracttype, Address, Env, String, Vec};

use crate::Error;

/// Cache TTL (24h, RedStone's own convention): how long a feed keeps serving
/// after submissions stop entirely. Deliberately looser than the aggregation
/// inclusion window below.
pub(crate) const DEFAULT_MAX_STALE_SECONDS: u64 = 86_400;

/// Aggregation inclusion window (15 min): a submission older than this counts
/// toward neither the median nor the reported observation time. Must be kept
/// `<=` every consumer's own `max_stale`.
pub(crate) const DEFAULT_MAX_SUBMISSION_AGE_SECONDS: u64 = 900;

/// Floor for `MaxSubmissionAgeSeconds`, so the window can't be set so tight
/// that ordinary propagation delay drops the quorum on every recompute.
pub(crate) const MIN_SUBMISSION_AGE_SECONDS: u64 = 60;

/// Default relative cluster skew equals the absolute inclusion window so a
/// desynced-but-still-fresh signer is not dropped beyond what
/// `MaxSubmissionAgeSeconds` already excludes. Tighten via
/// `set_max_relative_skew_seconds` when bots submit in a tight wave.
pub(crate) const DEFAULT_MAX_RELATIVE_SKEW_SECONDS: u64 = DEFAULT_MAX_SUBMISSION_AGE_SECONDS;

#[contracttype]
#[derive(Clone, Debug)]
pub(crate) enum DataKey {
    Signers,
    Threshold,
    MaxStaleSeconds,
    MaxSubmissionAgeSeconds,
    /// Max package-time lag behind the freshest absolute-fresh peer that may
    /// still enter the median cluster.
    MaxRelativeSkewSeconds,
    Resolution,
    LatestSubmission(String, Address),
    /// Per-signer index of feed ids the signer has submitted to. Lets
    /// `remove_signer` clean up in O(feeds-this-signer-touched).
    SignerFeeds(Address),
    CurrentAggregate(String),
    History(String),
    FeedMapping(ReflectorAsset),
    /// Reverse of `FeedMapping`: at most one asset may own a feed id.
    FeedOwner(String),
    /// Enumerable asset index: count, slot, and reverse lookup.
    AssetCount,
    AssetAt(u32),
    AssetIndex(ReflectorAsset),
    /// Enumerable known-feed allowlist: only registered feeds accept submits.
    /// Populated by `register_feed` / `add_feed`, not by raw submissions.
    FeedCount,
    FeedAt(u32),
    FeedIndex(String),
}

/// A single signer's latest raw submission for one feed. `price` stays `i128`
/// (not `U256`): per-signer submissions never leave the contract.
#[contracttype]
#[derive(Clone, Debug)]
pub(crate) struct SignerSubmission {
    pub(crate) price: i128,
    pub(crate) package_timestamp: u64,
}

pub(crate) fn load_signers(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&DataKey::Signers)
        .unwrap_or_else(|| Vec::new(env))
}

pub(crate) fn load_threshold(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::Threshold)
        .unwrap_or(0)
}

pub(crate) fn load_max_stale_seconds(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::MaxStaleSeconds)
        .unwrap_or(DEFAULT_MAX_STALE_SECONDS)
}

pub(crate) fn load_max_submission_age(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::MaxSubmissionAgeSeconds)
        .unwrap_or(DEFAULT_MAX_SUBMISSION_AGE_SECONDS)
}

/// Effective cluster skew, always capped by the absolute inclusion window.
pub(crate) fn load_max_relative_skew(env: &Env) -> u64 {
    let configured = env
        .storage()
        .instance()
        .get(&DataKey::MaxRelativeSkewSeconds)
        .unwrap_or(DEFAULT_MAX_RELATIVE_SKEW_SECONDS);
    configured.min(load_max_submission_age(env))
}

pub(crate) fn load_resolution(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::Resolution)
        .unwrap_or(0)
}

pub(crate) fn load_feed_id(env: &Env, asset: &ReflectorAsset) -> Option<String> {
    let key = DataKey::FeedMapping(asset.clone());
    env.storage().persistent().get(&key).inspect(|_| {
        renew_persistent_key(env, &key);
    })
}

pub(crate) fn load_feed_owner(env: &Env, feed_id: &String) -> Option<ReflectorAsset> {
    let key = DataKey::FeedOwner(feed_id.clone());
    env.storage().persistent().get(&key).inspect(|_| {
        renew_persistent_key(env, &key);
    })
}

// Read-renews slots so active index can't archive under a later swap-remove.
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

/// Inserts `feed_id` into the known-feed allowlist if absent; renews when present.
pub(crate) fn ensure_known_feed(env: &Env, feed_id: &String) {
    if !renew_known_feed(env, feed_id) {
        feed_index_insert(env, feed_id.clone());
    }
}

/// Renews the allowlist gate (`FeedIndex` + its paired `FeedAt` slot) for an
/// already-registered feed. Returns `false` when the entry is absent, without
/// inserting. Called on the submit hot path so an actively-submitted feed keeps
/// its gate alive on-chain rather than relying solely on the off-chain keeper.
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

pub(crate) fn load_signer_feeds(env: &Env, signer: &Address) -> Vec<String> {
    env.storage()
        .persistent()
        .get(&DataKey::SignerFeeds(signer.clone()))
        .unwrap_or_else(|| Vec::new(env))
}

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

fn asset_count(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::AssetCount)
        .unwrap_or(0)
}

fn feed_count(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::FeedCount)
        .unwrap_or(0)
}

pub(crate) fn feed_index_contains(env: &Env, feed_id: &String) -> bool {
    env.storage()
        .persistent()
        .has(&DataKey::FeedIndex(feed_id.clone()))
}

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

fn feed_index_insert(env: &Env, feed_id: String) {
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

/// Drops aggregate, history, per-signer submissions, known-feed index entry,
/// and reverse asset ownership for `feed_id`. Mapping for an asset is left to
/// the caller (`remove_feed`) so purge can reset data under a live mapping.
pub(crate) fn clear_feed_state(env: &Env, feed_id: &String) {
    env.storage()
        .persistent()
        .remove(&DataKey::CurrentAggregate(feed_id.clone()));
    env.storage()
        .persistent()
        .remove(&DataKey::History(feed_id.clone()));
    for signer in load_signers(env).iter() {
        env.storage()
            .persistent()
            .remove(&DataKey::LatestSubmission(feed_id.clone(), signer.clone()));
        remove_signer_feed(env, &signer, feed_id);
    }
    env.storage()
        .persistent()
        .remove(&DataKey::FeedOwner(feed_id.clone()));
    feed_index_remove(env, feed_id);
}

/// Reverts `NotAuthorizedSigner` when `signer` is not registered.
pub(crate) fn require_registered_signer(env: &Env, signer: &Address) -> Result<(), Error> {
    let signers = load_signers(env);
    if !signers.contains(signer) {
        return Err(Error::NotAuthorizedSigner);
    }
    Ok(())
}

/// Reverts `FeedNotKnown` when `feed_id` is absent from the allowlist.
pub(crate) fn require_known_feed(env: &Env, feed_id: &String) -> Result<(), Error> {
    if !feed_index_contains(env, feed_id) {
        return Err(Error::FeedNotKnown);
    }
    Ok(())
}

pub(crate) fn has_duplicate(signers: &Vec<Address>) -> bool {
    for i in 0..signers.len() {
        for j in (i + 1)..signers.len() {
            if signers.get(i).expect("invariant: i within signer vec len")
                == signers.get(j).expect("invariant: j within signer vec len")
            {
                return true;
            }
        }
    }
    false
}

pub(crate) fn renew_oracle_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD_INSTANCE, TTL_BUMP_INSTANCE);
}

pub(crate) fn renew_persistent_key(env: &Env, key: &DataKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}
