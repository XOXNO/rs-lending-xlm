use common::oracle::providers::reflector::ReflectorAsset;

use soroban_sdk::{contractimpl, Address, Env, String};

use stellar_macros::only_owner;

use crate::aggregation::recompute_aggregate;
use crate::storage::{
    asset_index_insert, asset_index_remove, clear_feed_state, ensure_known_feed,
    feed_index_contains, load_all_feeds, load_feed_owner, load_max_stale_seconds,
    load_max_submission_age, load_signer_feeds, load_signers, load_threshold,
    renew_oracle_instance, renew_persistent_key, DataKey, MIN_SUBMISSION_AGE_SECONDS,
};
use crate::{Error, XoxnoOracle, XoxnoOracleArgs, XoxnoOracleClient};

#[contractimpl]
impl XoxnoOracle {
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

        for feed_id in load_all_feeds(&env).iter() {
            recompute_aggregate(&env, &feed_id);
        }
        Ok(())
    }

    #[only_owner]
    pub fn set_max_stale_seconds(env: Env, seconds: u64) -> Result<(), Error> {
        renew_oracle_instance(&env);
        if seconds < load_max_submission_age(&env) {
            return Err(Error::InvalidSubmissionAge);
        }
        env.storage()
            .instance()
            .set(&DataKey::MaxStaleSeconds, &seconds);
        Ok(())
    }

    #[only_owner]
    pub fn set_max_submission_age_seconds(env: Env, seconds: u64) -> Result<(), Error> {
        renew_oracle_instance(&env);
        if seconds < MIN_SUBMISSION_AGE_SECONDS || seconds > load_max_stale_seconds(&env) {
            return Err(Error::InvalidSubmissionAge);
        }
        env.storage()
            .instance()
            .set(&DataKey::MaxSubmissionAgeSeconds, &seconds);

        for feed_id in load_all_feeds(&env).iter() {
            recompute_aggregate(&env, &feed_id);
        }
        Ok(())
    }

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

    #[only_owner]
    pub fn register_feed(env: Env, feed_id: String) -> Result<(), Error> {
        renew_oracle_instance(&env);
        if feed_index_contains(&env, &feed_id) {
            return Err(Error::FeedAlreadyRegistered);
        }
        ensure_known_feed(&env, &feed_id);
        Ok(())
    }

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

    #[only_owner]
    pub fn purge_feed(env: Env, feed_id: String) -> Result<(), Error> {
        renew_oracle_instance(&env);

        if !feed_index_contains(&env, &feed_id) {
            return Err(Error::FeedNotKnown);
        }

        if let Some(asset) = load_feed_owner(&env, &feed_id) {
            let map_key = DataKey::FeedMapping(asset.clone());
            env.storage().persistent().remove(&map_key);
            asset_index_remove(&env, &asset);
        }

        clear_feed_state(&env, &feed_id);
        Ok(())
    }
}
