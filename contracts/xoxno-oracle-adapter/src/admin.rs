//! Owner-gated entrypoints: signer-set, threshold, staleness windows,
//! feed-mapping, resolution, and feed purge. Owner auth comes from
//! `stellar_access::ownable` (see `lib.rs`).

use common::oracle::providers::reflector::ReflectorAsset;
use soroban_sdk::{contractimpl, Address, Env, String};
use stellar_macros::only_owner;

use crate::aggregation::recompute_aggregate;
use crate::storage::{
    asset_index_insert, asset_index_remove, feed_index_contains, feed_index_remove, load_all_feeds,
    load_max_stale_seconds, load_max_submission_age, load_signer_feeds, load_signers,
    load_threshold, remove_signer_feed, renew_oracle_instance, renew_persistent_key, DataKey,
    MIN_SUBMISSION_AGE_SECONDS,
};
use crate::{Error, XoxnoOracle, XoxnoOracleArgs, XoxnoOracleClient};

#[contractimpl]
impl XoxnoOracle {
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
    /// per-feed submissions across every feed it touched.
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

        // Re-validate every known feed so an aggregate computed under a lower
        // threshold can't keep serving. O(known-feeds); infrequent admin op.
        for feed_id in load_all_feeds(&env).iter() {
            recompute_aggregate(&env, &feed_id);
        }
        Ok(())
    }

    /// Sets the cache-TTL ceiling (seconds) applied to cached aggregates in
    /// `read_price_data_for_feed`. Must stay `>=` the aggregation inclusion
    /// window (`MaxSubmissionAgeSeconds`).
    ///
    /// # Errors
    /// * `InvalidSubmissionAge` - `seconds < MaxSubmissionAgeSeconds`.
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

    /// Sets the aggregation inclusion window (seconds): a submission older
    /// than this is excluded from both the median and the reported
    /// observation time. Keep this `<=` every consumer's `max_stale`.
    ///
    /// # Errors
    /// * `InvalidSubmissionAge` - `seconds < MIN_SUBMISSION_AGE_SECONDS` or
    ///   `seconds > MaxStaleSeconds`.
    #[only_owner]
    pub fn set_max_submission_age_seconds(env: Env, seconds: u64) -> Result<(), Error> {
        renew_oracle_instance(&env);
        if seconds < MIN_SUBMISSION_AGE_SECONDS || seconds > load_max_stale_seconds(&env) {
            return Err(Error::InvalidSubmissionAge);
        }
        env.storage()
            .instance()
            .set(&DataKey::MaxSubmissionAgeSeconds, &seconds);

        // Tightening the window can push a previously-included submission out
        // of range: recompute every known feed so a stale aggregate clears now
        // instead of serving until the next submission.
        for feed_id in load_all_feeds(&env).iter() {
            recompute_aggregate(&env, &feed_id);
        }
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

    /// Removes `asset`'s feed mapping and drops it from the asset index.
    /// Submission-side storage for the mapped feed id stays; use `purge_feed`
    /// to reclaim that.
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

    /// Purges all submission-side storage for a retired `feed_id`: aggregate,
    /// history, every registered signer's submission and `SignerFeeds` trace,
    /// and the known-feed index entry. `FeedMapping`/the asset index are
    /// untouched — use `remove_feed` for those.
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
                .remove(&DataKey::LatestSubmission(feed_id.clone(), signer.clone()));
            // Keep `SignerFeeds` consistent with the known-feed set, else the
            // purged feed lingers there across purge/re-add cycles.
            remove_signer_feed(&env, &signer, &feed_id);
        }

        feed_index_remove(&env, &feed_id);
        Ok(())
    }
}
