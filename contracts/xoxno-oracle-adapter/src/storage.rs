//! Storage keys, raw storage helpers, and owner-gated entrypoints
//! (signer-set, threshold, staleness, feed-mapping, and resolution
//! management). Owner auth itself lives in `stellar_access::ownable`
//! (see `lib.rs`'s `Ownable` impl) — this module only stores oracle-specific
//! state.
//!
//! Assets and known-feed-ids are each kept as a swap-remove indexed set
//! (count + slot-array + reverse lookup) in persistent storage rather than a
//! single growing `Vec` in instance storage, so add/remove cost O(1)
//! persistent writes instead of rewriting an ever-larger blob that instance
//! storage would otherwise rehydrate on every contract call.

use common::constants::{
    TTL_BUMP_INSTANCE, TTL_BUMP_SHARED, TTL_THRESHOLD_INSTANCE, TTL_THRESHOLD_SHARED,
};
use common::oracle::providers::reflector::ReflectorAsset;
use soroban_sdk::{contractimpl, contracttype, Address, Env, String, Vec};
use stellar_macros::only_owner;

use crate::aggregation::recompute_aggregate;
use crate::{Error, XoxnoOracle, XoxnoOracleArgs, XoxnoOracleClient};

/// Real RedStone's own staleness convention for cache freshness (24 hours).
pub(crate) const DEFAULT_MAX_STALE_SECONDS: u64 = 86_400;

#[contracttype]
#[derive(Clone, Debug)]
pub(crate) enum DataKey {
    Signers,
    Threshold,
    MaxStaleSeconds,
    Resolution,
    LatestSubmission(String, Address),
    /// Per-signer index of the feed ids that signer has submitted to. Lets
    /// `remove_signer` clean up in O(feeds-this-signer-touched) instead of
    /// O(all-feeds-ever-seen).
    SignerFeeds(Address),
    CurrentAggregate(String),
    History(String),
    FeedMapping(ReflectorAsset),
    /// Enumerable asset index (see module docs): count, slot, and reverse
    /// lookup for `AllAssets`.
    AssetCount,
    AssetAt(u32),
    AssetIndex(ReflectorAsset),
    /// Enumerable feed index (see module docs) backing `KnownFeeds`: every
    /// feed id that has ever received a `submit_price`/`submit_prices` call,
    /// regardless of whether it is also mapped in `FeedMapping`. Backs
    /// `remove_signer`'s orphan cleanup and `purge_feed`.
    FeedCount,
    FeedAt(u32),
    FeedIndex(String),
}

/// A single signer's latest raw submission for one feed. `price` is kept as
/// `i128` (not `U256`) here since only the aggregated median is ever exposed
/// externally; per-signer submissions never leave the contract.
#[contracttype]
#[derive(Clone, Debug)]
pub(crate) struct SignerSubmission {
    pub(crate) price: i128,
    pub(crate) package_timestamp: u64,
}

#[contractimpl]
impl XoxnoOracle {
    // -----------------------------------------------------------------
    // Owner functions — gated by `#[only_owner]` (stellar_access::ownable).
    // -----------------------------------------------------------------

    /// Adds `signer` to the registered signer set.
    ///
    /// # Errors
    /// * `SignerAlreadyRegistered` - `signer` is already registered.
    #[only_owner]
    pub fn add_signer(env: Env, signer: Address) -> Result<(), Error> {
        renew_oracle_instance(&env);
        let mut signers = load_signers(&env);
        if signers.contains(&signer) {
            return Err(Error::SignerAlreadyRegistered);
        }
        signers.push_back(signer);
        env.storage().instance().set(&DataKey::Signers, &signers);
        Ok(())
    }

    /// Removes `signer` from the registered signer set and purges its
    /// per-feed submissions across every known feed.
    ///
    /// # Errors
    /// * `SignerNotRegistered` - `signer` is not registered.
    /// * `CannotRemoveBelowThreshold` - removing `signer` would drop the
    ///   signer count below the current threshold.
    #[only_owner]
    pub fn remove_signer(env: Env, signer: Address) -> Result<(), Error> {
        renew_oracle_instance(&env);
        let mut signers = load_signers(&env);
        let index = signers.iter().position(|s| s == signer);
        let Some(index) = index else {
            return Err(Error::SignerNotRegistered);
        };

        let threshold = load_threshold(&env);
        if signers.len() - 1 < threshold {
            return Err(Error::CannotRemoveBelowThreshold);
        }

        signers.remove(index as u32);
        env.storage().instance().set(&DataKey::Signers, &signers);

        // Purge only the feeds this signer actually submitted to — cost is
        // O(feeds-this-signer-touched), not O(all-feeds-ever-seen) — and
        // recompute each affected aggregate over the now-remaining signers so a
        // value this signer poisoned into `CurrentAggregate` is evicted
        // immediately rather than serving until `MaxStaleSeconds`.
        for feed_id in load_signer_feeds(&env, &signer).iter() {
            env.storage()
                .persistent()
                .remove(&DataKey::LatestSubmission(feed_id.clone(), signer.clone()));
            recompute_aggregate(&env, &feed_id);
        }
        env.storage()
            .persistent()
            .remove(&DataKey::SignerFeeds(signer));
        Ok(())
    }

    /// Sets the N-of-M aggregation threshold.
    ///
    /// # Errors
    /// * `InvalidThreshold` - `threshold == 0` or `threshold` exceeds the
    ///   current signer count.
    #[only_owner]
    pub fn set_threshold(env: Env, threshold: u32) -> Result<(), Error> {
        renew_oracle_instance(&env);
        let signers = load_signers(&env);
        if threshold == 0 || threshold > signers.len() {
            return Err(Error::InvalidThreshold);
        }
        env.storage()
            .instance()
            .set(&DataKey::Threshold, &threshold);

        // Re-validate every known feed's cached aggregate against the new
        // threshold: feeds that no longer meet quorum are cleared by
        // `recompute_aggregate`'s fail-safe branch, so a value computed under an
        // old lower threshold can't keep serving. O(known-feeds); acceptable for
        // an infrequent admin op.
        for feed_id in load_all_feeds(&env).iter() {
            recompute_aggregate(&env, &feed_id);
        }
        Ok(())
    }

    /// Sets the absolute staleness ceiling (seconds) applied to cached
    /// aggregates in `read_price_data_for_feed`.
    #[only_owner]
    pub fn set_max_stale_seconds(env: Env, seconds: u64) -> Result<(), Error> {
        renew_oracle_instance(&env);
        env.storage()
            .instance()
            .set(&DataKey::MaxStaleSeconds, &seconds);
        Ok(())
    }

    /// Maps `asset` to `feed_id` for the SEP-40 reader endpoints and adds it
    /// to the enumerable asset index.
    ///
    /// # Errors
    /// * `FeedAlreadyMapped` - `asset` already has a feed mapping.
    #[only_owner]
    pub fn add_feed(env: Env, feed_id: String, asset: ReflectorAsset) -> Result<(), Error> {
        renew_oracle_instance(&env);
        let key = DataKey::FeedMapping(asset.clone());
        if env.storage().persistent().has(&key) {
            return Err(Error::FeedAlreadyMapped);
        }
        env.storage().persistent().set(&key, &feed_id);
        renew_persistent_key(&env, &key);

        asset_index_insert(&env, asset);
        Ok(())
    }

    /// Removes `asset`'s feed mapping and drops it from the asset index. Does
    /// not touch the known-feed index or any submission-side storage for the
    /// feed id it was mapped to; use `purge_feed` to reclaim that.
    ///
    /// # Errors
    /// * `FeedNotMapped` - `asset` has no feed mapping.
    #[only_owner]
    pub fn remove_feed(env: Env, asset: ReflectorAsset) -> Result<(), Error> {
        renew_oracle_instance(&env);
        let key = DataKey::FeedMapping(asset.clone());
        if !env.storage().persistent().has(&key) {
            return Err(Error::FeedNotMapped);
        }
        env.storage().persistent().remove(&key);

        asset_index_remove(&env, &asset);
        Ok(())
    }

    /// Sets the TWAP resolution reported by `resolution()`.
    #[only_owner]
    pub fn set_resolution(env: Env, resolution: u32) -> Result<(), Error> {
        renew_oracle_instance(&env);
        env.storage()
            .instance()
            .set(&DataKey::Resolution, &resolution);
        Ok(())
    }

    /// Purges all submission-side storage for a retired `feed_id`:
    /// `CurrentAggregate`, `History`, every currently-registered signer's
    /// `LatestSubmission` entry, and its known-feed index entry. Does not
    /// touch `FeedMapping`/the asset index — use `remove_feed` for that.
    ///
    /// # Errors
    /// * `FeedNotKnown` - `feed_id` has never received a submission.
    #[only_owner]
    pub fn purge_feed(env: Env, feed_id: String) -> Result<(), Error> {
        renew_oracle_instance(&env);

        if !feed_index_contains(&env, &feed_id) {
            return Err(Error::FeedNotKnown);
        }

        env.storage()
            .persistent()
            .remove(&DataKey::CurrentAggregate(feed_id.clone()));
        env.storage()
            .persistent()
            .remove(&DataKey::History(feed_id.clone()));
        for signer in load_signers(&env).iter() {
            env.storage()
                .persistent()
                .remove(&DataKey::LatestSubmission(feed_id.clone(), signer));
        }

        feed_index_remove(&env, &feed_id);
        Ok(())
    }
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

/// Materializes the asset index into a `Vec`, for `assets()`'s external
/// contract signature.
pub(crate) fn load_all_assets(env: &Env) -> Vec<ReflectorAsset> {
    let count = asset_count(env);
    let mut out = Vec::new(env);
    for i in 0..count {
        let key = DataKey::AssetAt(i);
        if let Some(asset) = env.storage().persistent().get(&key) {
            // Read-renewal: keep enumerated slots alive so an active index can't
            // archive under normal reads and later trap a swap-remove partner
            // read.
            renew_persistent_key(env, &key);
            out.push_back(asset);
        }
    }
    out
}

/// Materializes the known-feed index into a `Vec` of every feed id that has
/// ever received a submission. Read-renews each enumerated slot so an active
/// index can't archive under this read and later trap a swap-remove partner
/// read.
fn load_all_feeds(env: &Env) -> Vec<String> {
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

pub(crate) fn load_feed_id(env: &Env, asset: &ReflectorAsset) -> Option<String> {
    env.storage()
        .persistent()
        .get(&DataKey::FeedMapping(asset.clone()))
}

/// Records `feed_id` in the known-feed index the first time it is ever
/// submitted to. On later calls for an already-known feed it renews that feed's
/// index slots (read-renewal on the hot submit path) so an actively-submitted
/// feed's index entries can't archive and later trap `feed_index_remove`'s
/// swap-remove partner read.
pub(crate) fn record_known_feed(env: &Env, feed_id: &String) {
    let index_key = DataKey::FeedIndex(feed_id.clone());
    match env.storage().persistent().get::<DataKey, u32>(&index_key) {
        Some(slot) => {
            renew_persistent_key(env, &index_key);
            renew_persistent_key(env, &DataKey::FeedAt(slot));
        }
        None => feed_index_insert(env, feed_id.clone()),
    }
}

/// Adds `feed_id` to `signer`'s per-signer feed index on its first submission
/// to that feed; idempotent on later submissions (renewing the entry's TTL).
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

fn load_signer_feeds(env: &Env, signer: &Address) -> Vec<String> {
    env.storage()
        .persistent()
        .get(&DataKey::SignerFeeds(signer.clone()))
        .unwrap_or_else(|| Vec::new(env))
}

fn asset_count(env: &Env) -> u32 {
    env.storage().persistent().get(&DataKey::AssetCount).unwrap_or(0)
}

fn feed_count(env: &Env) -> u32 {
    env.storage().persistent().get(&DataKey::FeedCount).unwrap_or(0)
}

fn feed_index_contains(env: &Env, feed_id: &String) -> bool {
    env.storage()
        .persistent()
        .has(&DataKey::FeedIndex(feed_id.clone()))
}

/// Appends `asset` as the last slot and records its slot in the reverse
/// index, so membership checks and removal are both O(1).
fn asset_index_insert(env: &Env, asset: ReflectorAsset) {
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

/// Swap-removes `asset`: moves the last slot into the removed slot's place
/// (updating that moved asset's reverse index) and shrinks the count, so no
/// gap is left in `AssetAt` and no full-index rewrite is needed. A no-op if
/// `asset` is not present.
fn asset_index_remove(env: &Env, asset: &ReflectorAsset) {
    let index_key = DataKey::AssetIndex(asset.clone());
    let Some(removed_at): Option<u32> = env.storage().persistent().get(&index_key) else {
        return;
    };
    env.storage().persistent().remove(&index_key);

    let count = asset_count(env);
    let last_at = count - 1;
    if removed_at != last_at {
        let last_key = DataKey::AssetAt(last_at);
        // safe: last_at = count-1 and slots 0..count are always populated by the
        // index invariant; `load_all_assets` read-renews them so an active slot
        // can't archive out from under this read.
        let moved: ReflectorAsset = env.storage().persistent().get(&last_key).unwrap();
        let moved_at_key = DataKey::AssetAt(removed_at);
        env.storage().persistent().set(&moved_at_key, &moved);
        renew_persistent_key(env, &moved_at_key);

        let moved_index_key = DataKey::AssetIndex(moved);
        env.storage().persistent().set(&moved_index_key, &removed_at);
        renew_persistent_key(env, &moved_index_key);
    }
    env.storage().persistent().remove(&DataKey::AssetAt(last_at));

    let count_key = DataKey::AssetCount;
    env.storage().persistent().set(&count_key, &last_at);
    renew_persistent_key(env, &count_key);
}

/// Appends `feed_id` as the last slot; see `asset_index_insert` for the
/// swap-remove index shape this mirrors.
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

/// Swap-removes `feed_id`; see `asset_index_remove` for the mechanics. A
/// no-op if `feed_id` is not present.
fn feed_index_remove(env: &Env, feed_id: &String) {
    let index_key = DataKey::FeedIndex(feed_id.clone());
    let Some(removed_at): Option<u32> = env.storage().persistent().get(&index_key) else {
        return;
    };
    env.storage().persistent().remove(&index_key);

    let count = feed_count(env);
    let last_at = count - 1;
    if removed_at != last_at {
        let last_key = DataKey::FeedAt(last_at);
        // safe: last_at = count-1 and slots 0..count are always populated by the
        // index invariant; `record_known_feed` read-renews an active feed's
        // slots so they can't archive out from under this read.
        let moved: String = env.storage().persistent().get(&last_key).unwrap();
        let moved_at_key = DataKey::FeedAt(removed_at);
        env.storage().persistent().set(&moved_at_key, &moved);
        renew_persistent_key(env, &moved_at_key);

        let moved_index_key = DataKey::FeedIndex(moved);
        env.storage().persistent().set(&moved_index_key, &removed_at);
        renew_persistent_key(env, &moved_index_key);
    }
    env.storage().persistent().remove(&DataKey::FeedAt(last_at));

    let count_key = DataKey::FeedCount;
    env.storage().persistent().set(&count_key, &last_at);
    renew_persistent_key(env, &count_key);
}

pub(crate) fn require_registered_signer(env: &Env, signer: &Address) -> Result<(), Error> {
    let signers = load_signers(env);
    if !signers.contains(signer) {
        return Err(Error::NotAuthorizedSigner);
    }
    Ok(())
}

pub(crate) fn has_duplicate(signers: &Vec<Address>) -> bool {
    for i in 0..signers.len() {
        for j in (i + 1)..signers.len() {
            if signers.get(i).unwrap() == signers.get(j).unwrap() {
                return true;
            }
        }
    }
    false
}

/// Extends this contract's instance-storage TTL.
pub(crate) fn renew_oracle_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD_INSTANCE, TTL_BUMP_INSTANCE);
}

/// Extends the shared-tier TTL on a persistent key meant to live indefinitely
/// (`FeedMapping`, `CurrentAggregate`, `History`, `LatestSubmission`).
pub(crate) fn renew_persistent_key(env: &Env, key: &DataKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}
