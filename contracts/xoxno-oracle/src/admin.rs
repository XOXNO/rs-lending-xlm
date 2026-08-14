//! Owner-only administrative entry points for the oracle contract: signer
//! and threshold management, staleness and skew bound configuration, feed
//! registration and asset mapping, and price resolution.

use common::oracle::providers::reflector::ReflectorAsset;

use soroban_sdk::{contractimpl, Address, Env, String};

use stellar_macros::only_owner;

use crate::aggregation::recompute_aggregate;
use crate::storage::{
    asset_index_insert, asset_index_remove, clear_feed_state, clear_signer_feeds,
    ensure_known_feed, feed_index_contains, has_feed_mapping, load_all_feeds, load_feed_owner,
    load_max_stale_seconds, load_max_submission_age, load_signer_feeds, load_signers,
    load_threshold, map_feed, remove_feed_mapping, remove_submission, renew_oracle_instance,
    store_max_relative_skew, store_max_stale_seconds, store_max_submission_age, store_resolution,
    store_signers, store_threshold, take_feed_mapping, MIN_SUBMISSION_AGE_SECONDS,
};
use crate::{Error, XoxnoOracle, XoxnoOracleArgs, XoxnoOracleClient};

#[contractimpl]
impl XoxnoOracle {
    /// Adds `signer` to the set of addresses authorized to submit prices.
    /// Fails with `SignerAlreadyRegistered` if the address is already present.
    #[only_owner]
    pub fn add_signer(env: Env, signer: Address) -> Result<(), Error> {
        renew_oracle_instance(&env);
        let mut signers = load_signers(&env);
        if signers.contains(&signer) {
            return Err(Error::SignerAlreadyRegistered);
        }
        signers.push_back(signer);
        store_signers(&env, &signers);
        Ok(())
    }

    /// Removes `signer` from the signer set. Fails with `SignerNotRegistered`
    /// if the address is not currently a signer, and with
    /// `CannotRemoveBelowThreshold` if removal would drop the signer count
    /// below the configured threshold. Deletes the signer's latest submission
    /// for every feed it had submitted to, recomputes the aggregate for each
    /// of those feeds, and clears the signer's feed list.
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
        store_signers(&env, &signers);

        for feed_id in load_signer_feeds(&env, &signer).iter() {
            remove_submission(&env, &feed_id, &signer);
            recompute_aggregate(&env, &feed_id);
        }
        clear_signer_feeds(&env, &signer);
        Ok(())
    }

    /// Sets the minimum number of signer submissions required to accept a
    /// price for a feed. Fails with `InvalidThreshold` if `threshold` is zero
    /// or exceeds the current signer count. Recomputes the aggregate for
    /// every known feed under the new threshold.
    #[only_owner]
    pub fn set_threshold(env: Env, threshold: u32) -> Result<(), Error> {
        renew_oracle_instance(&env);
        let signers = load_signers(&env);
        if threshold == 0 || threshold > signers.len() {
            return Err(Error::InvalidThreshold);
        }
        store_threshold(&env, threshold);

        for feed_id in load_all_feeds(&env).iter() {
            recompute_aggregate(&env, &feed_id);
        }
        Ok(())
    }

    /// Sets the maximum age, in seconds, a stored aggregate can reach before
    /// reads treat it as stale. Fails with `InvalidSubmissionAge` if `seconds`
    /// is smaller than the configured maximum submission age.
    #[only_owner]
    pub fn set_max_stale_seconds(env: Env, seconds: u64) -> Result<(), Error> {
        renew_oracle_instance(&env);
        if seconds < load_max_submission_age(&env) {
            return Err(Error::InvalidSubmissionAge);
        }
        store_max_stale_seconds(&env, seconds);
        Ok(())
    }

    /// Sets the maximum age, in seconds, a signer's submission timestamp can
    /// have relative to ledger time to be accepted. Fails with
    /// `InvalidSubmissionAge` if `seconds` is below `MIN_SUBMISSION_AGE_SECONDS`
    /// or above the configured maximum stale age. Recomputes the aggregate
    /// for every known feed under the new bound.
    #[only_owner]
    pub fn set_max_submission_age_seconds(env: Env, seconds: u64) -> Result<(), Error> {
        renew_oracle_instance(&env);
        if seconds < MIN_SUBMISSION_AGE_SECONDS || seconds > load_max_stale_seconds(&env) {
            return Err(Error::InvalidSubmissionAge);
        }
        store_max_submission_age(&env, seconds);

        for feed_id in load_all_feeds(&env).iter() {
            recompute_aggregate(&env, &feed_id);
        }
        Ok(())
    }

    /// Sets the maximum allowed timestamp skew, in seconds, between clustered
    /// signer submissions for the same aggregate. Fails with
    /// `InvalidRelativeSkew` if `seconds` exceeds the configured maximum
    /// submission age. Recomputes the aggregate for every known feed under
    /// the new bound.
    #[only_owner]
    pub fn set_max_relative_skew_seconds(env: Env, seconds: u64) -> Result<(), Error> {
        renew_oracle_instance(&env);
        if seconds > load_max_submission_age(&env)
            || seconds <= common::oracle::observation::MAX_FUTURE_SKEW_SECONDS
        {
            return Err(Error::InvalidRelativeSkew);
        }
        store_max_relative_skew(&env, seconds);

        for feed_id in load_all_feeds(&env).iter() {
            recompute_aggregate(&env, &feed_id);
        }
        Ok(())
    }

    /// Registers `feed_id` as a known feed without mapping it to a
    /// `ReflectorAsset`. Fails with `FeedAlreadyRegistered` if the feed is
    /// already known.
    #[only_owner]
    pub fn register_feed(env: Env, feed_id: String) -> Result<(), Error> {
        renew_oracle_instance(&env);
        if feed_index_contains(&env, &feed_id) {
            return Err(Error::FeedAlreadyRegistered);
        }
        ensure_known_feed(&env, &feed_id);
        Ok(())
    }

    /// Maps `asset` to `feed_id` in both directions and registers `feed_id`
    /// as known. Fails with `FeedAlreadyMapped` if `asset` already has a feed
    /// mapping or `feed_id` already has an owning asset.
    #[only_owner]
    pub fn add_feed(env: Env, feed_id: String, asset: ReflectorAsset) -> Result<(), Error> {
        renew_oracle_instance(&env);
        if has_feed_mapping(&env, &asset) {
            return Err(Error::FeedAlreadyMapped);
        }
        if load_feed_owner(&env, &feed_id).is_some() {
            return Err(Error::FeedAlreadyMapped);
        }
        map_feed(&env, &asset, &feed_id);

        ensure_known_feed(&env, &feed_id);
        asset_index_insert(&env, asset);
        Ok(())
    }

    /// Removes the feed mapping owned by `asset` and clears all stored state
    /// for the underlying feed (aggregate, history, per-signer submissions,
    /// and feed index entry). Fails with `FeedNotMapped` if `asset` has no
    /// feed mapping.
    #[only_owner]
    pub fn remove_feed(env: Env, asset: ReflectorAsset) -> Result<(), Error> {
        renew_oracle_instance(&env);
        let Some(feed_id) = take_feed_mapping(&env, &asset) else {
            return Err(Error::FeedNotMapped);
        };
        asset_index_remove(&env, &asset);
        clear_feed_state(&env, &feed_id);
        Ok(())
    }

    /// Sets the price resolution, in seconds, used to decide whether a new
    /// aggregate replaces or appends to the last history entry.
    #[only_owner]
    pub fn set_resolution(env: Env, resolution: u32) -> Result<(), Error> {
        renew_oracle_instance(&env);
        store_resolution(&env, resolution);
        Ok(())
    }

    /// Removes `feed_id` and all its stored state, including its asset
    /// mapping if one exists. Fails with `FeedNotKnown` if the feed is not
    /// registered.
    #[only_owner]
    pub fn purge_feed(env: Env, feed_id: String) -> Result<(), Error> {
        renew_oracle_instance(&env);

        if !feed_index_contains(&env, &feed_id) {
            return Err(Error::FeedNotKnown);
        }

        if let Some(asset) = load_feed_owner(&env, &feed_id) {
            remove_feed_mapping(&env, &asset);
            asset_index_remove(&env, &asset);
        }

        clear_feed_state(&env, &feed_id);
        Ok(())
    }
}
