//! Owner-gated entrypoints: signer-set, threshold, staleness windows,
//! feed-mapping, resolution, and feed purge. Owner auth comes from
//! `stellar_access::ownable` (see `lib.rs`).

use common::oracle::providers::reflector::ReflectorAsset;

use soroban_sdk::{contractimpl, Address, Env, String};

use stellar_macros::only_owner;

use crate::aggregation::recompute_aggregate;
use crate::storage::{
    asset_index_insert, asset_index_remove, clear_feed_state, ensure_known_feed, feed_index_contains,
    load_all_feeds, load_feed_owner, load_max_stale_seconds, load_max_submission_age,
    load_signer_feeds, load_signers, load_threshold, renew_oracle_instance, renew_persistent_key,
    DataKey, MIN_SUBMISSION_AGE_SECONDS,
};
use crate::{Error, XoxnoOracle, XoxnoOracleArgs, XoxnoOracleClient};

#[contractimpl]
impl XoxnoOracle {
    /// # Errors
    /// * `SignerAlreadyRegistered`
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

    /// Side effects: drops signer's submissions; recomputes each touched feed.
    ///
    /// # Errors
    /// * `SignerNotRegistered`
    /// * `CannotRemoveBelowThreshold`
    #[only_owner]
    pub fn remove_signer(env: Env, signer: Address) -> Result<(), Error> {
        renew_oracle_instance(&env);
        let mut signers = load_signers(&env);
        let Some(index) = signers.first_index_of(&signer) else {
            return Err(Error::SignerNotRegistered);
        };

        let threshold = load_threshold(&env);
        if signers.len() - 1 < threshold {
            return Err(Error::CannotRemoveBelowThreshold);
        }

        signers.remove(index);
        env.storage().instance().set(&DataKey::Signers, &signers);

        // Recompute each affected feed so a value this signer poisoned into
        // `CurrentAggregate` is evicted immediately, not at `MaxStaleSeconds`.
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

    /// Side effects: recomputes every known feed under the new threshold.
    ///
    /// # Errors
    /// * `InvalidThreshold` - zero or above signer count
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

        // Re-validate every known feed so an aggregate computed under a lower
        // threshold can't keep serving. O(known-feeds); infrequent admin op.
        for feed_id in load_all_feeds(&env).iter() {
            recompute_aggregate(&env, &feed_id);
        }
        Ok(())
    }

    /// Cache-TTL ceiling; must stay `>= MaxSubmissionAgeSeconds`. No recompute.
    ///
    /// # Errors
    /// * `InvalidSubmissionAge`
    #[only_owner]
    pub fn set_max_stale_seconds(env: Env, seconds: u64) -> Result<(), Error> {
        renew_oracle_instance(&env);
        if seconds < load_max_submission_age(&env) {
            return Err(Error::InvalidSubmissionAge);
        }
        env.storage()
            .instance()
            .set(&DataKey::MaxStaleSeconds, &seconds);
        // No recompute: the TTL is re-evaluated live on every read; no cached
        // state depends on it.
        Ok(())
    }

    /// Inclusion window for median + observation time. Keep `<=` consumer max_stale.
    /// Side effects: recomputes all feeds.
    ///
    /// # Errors
    /// * `InvalidSubmissionAge` - below floor or above MaxStaleSeconds
    #[only_owner]
    pub fn set_max_submission_age_seconds(env: Env, seconds: u64) -> Result<(), Error> {
        renew_oracle_instance(&env);
        if seconds < MIN_SUBMISSION_AGE_SECONDS || seconds > load_max_stale_seconds(&env) {
            return Err(Error::InvalidSubmissionAge);
        }
        env.storage()
            .instance()
            .set(&DataKey::MaxSubmissionAgeSeconds, &seconds);

        // Tighter age window may invalidate in-range submissions; recompute all feeds.
        for feed_id in load_all_feeds(&env).iter() {
            recompute_aggregate(&env, &feed_id);
        }
        Ok(())
    }

    /// Max package-time lag behind the freshest absolute-fresh peer that may
    /// still enter the median cluster. Capped by `MaxSubmissionAgeSeconds`.
    /// Side effects: recomputes all feeds.
    ///
    /// # Errors
    /// * `InvalidRelativeSkew` - above MaxSubmissionAgeSeconds
    #[only_owner]
    pub fn set_max_relative_skew_seconds(env: Env, seconds: u64) -> Result<(), Error> {
        renew_oracle_instance(&env);
        if seconds > load_max_submission_age(&env) {
            return Err(Error::InvalidRelativeSkew);
        }
        env.storage()
            .instance()
            .set(&DataKey::MaxRelativeSkewSeconds, &seconds);

        for feed_id in load_all_feeds(&env).iter() {
            recompute_aggregate(&env, &feed_id);
        }
        Ok(())
    }

    /// Owner allowlist for a RedStone-style `feed_id` without SEP-40 mapping.
    /// Submissions to unregistered feed ids are rejected.
    ///
    /// # Errors
    /// * `FeedAlreadyRegistered`
    #[only_owner]
    pub fn register_feed(env: Env, feed_id: String) -> Result<(), Error> {
        renew_oracle_instance(&env);
        if feed_index_contains(&env, &feed_id) {
            return Err(Error::FeedAlreadyRegistered);
        }
        ensure_known_feed(&env, &feed_id);
        Ok(())
    }

    /// Maps `asset` → `feed_id` for SEP-40 reads and ensures the feed is on the
    /// submit allowlist. At most one asset may own a given feed id.
    ///
    /// # Errors
    /// * `FeedAlreadyMapped` - asset already mapped, or feed id already owned
    #[only_owner]
    pub fn add_feed(env: Env, feed_id: String, asset: ReflectorAsset) -> Result<(), Error> {
        renew_oracle_instance(&env);
        let key = DataKey::FeedMapping(asset.clone());
        if env.storage().persistent().has(&key) {
            return Err(Error::FeedAlreadyMapped);
        }
        if load_feed_owner(&env, &feed_id).is_some() {
            return Err(Error::FeedAlreadyMapped);
        }
        env.storage().persistent().set(&key, &feed_id);
        renew_persistent_key(&env, &key);

        let owner_key = DataKey::FeedOwner(feed_id.clone());
        env.storage().persistent().set(&owner_key, &asset);
        renew_persistent_key(&env, &owner_key);

        ensure_known_feed(&env, &feed_id);
        asset_index_insert(&env, asset);
        Ok(())
    }

    /// Drops SEP-40 mapping and wipes all price state for the mapped feed
    /// (aggregate, history, submissions, allowlist entry).
    ///
    /// # Errors
    /// * `FeedNotMapped`
    #[only_owner]
    pub fn remove_feed(env: Env, asset: ReflectorAsset) -> Result<(), Error> {
        renew_oracle_instance(&env);
        let key = DataKey::FeedMapping(asset.clone());
        let Some(feed_id) = env.storage().persistent().get::<DataKey, String>(&key) else {
            return Err(Error::FeedNotMapped);
        };
        env.storage().persistent().remove(&key);
        asset_index_remove(&env, &asset);
        clear_feed_state(&env, &feed_id);
        Ok(())
    }

    #[only_owner]
    pub fn set_resolution(env: Env, resolution: u32) -> Result<(), Error> {
        renew_oracle_instance(&env);
        env.storage()
            .instance()
            .set(&DataKey::Resolution, &resolution);
        Ok(())
    }

    /// Clears aggregate, history, per-signer submissions, known-feed allowlist
    /// entry, and reverse ownership. Does not touch a residual asset mapping
    /// if called after `remove_feed` already dropped it; when a mapping still
    /// exists, the owner should call `remove_feed` instead so indexes stay
    /// consistent. Prefer `remove_feed` for full teardown.
    ///
    /// # Errors
    /// * `FeedNotKnown`
    #[only_owner]
    pub fn purge_feed(env: Env, feed_id: String) -> Result<(), Error> {
        renew_oracle_instance(&env);

        if !feed_index_contains(&env, &feed_id) {
            return Err(Error::FeedNotKnown);
        }

        // If an asset still owns this feed, drop that mapping + asset index so
        // SEP-40 and reverse ownership cannot point at a wiped feed.
        if let Some(asset) = load_feed_owner(&env, &feed_id) {
            let map_key = DataKey::FeedMapping(asset.clone());
            env.storage().persistent().remove(&map_key);
            asset_index_remove(&env, &asset);
        }

        clear_feed_state(&env, &feed_id);
        Ok(())
    }
}
