//! Reads and writes instance-storage configuration values for the oracle:
//! the signer set, submission threshold, staleness and skew bounds, and
//! price resolution. Every read accessor falls back to a default when the
//! underlying key is absent from instance storage.

use soroban_sdk::{Address, Env, Vec};

use crate::storage::{
    DataKey, DEFAULT_MAX_RELATIVE_SKEW_SECONDS, DEFAULT_MAX_STALE_SECONDS,
    DEFAULT_MAX_SUBMISSION_AGE_SECONDS,
};

/// Loads the configured signer addresses. Returns an empty vector if none are set.
pub(crate) fn load_signers(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&DataKey::Signers)
        .unwrap_or_else(|| Vec::new(env))
}

/// Overwrites the configured signer addresses.
pub(crate) fn store_signers(env: &Env, signers: &Vec<Address>) {
    env.storage().instance().set(&DataKey::Signers, signers);
}

/// Overwrites the minimum number of signer submissions required to accept a price.
pub(crate) fn store_threshold(env: &Env, threshold: u32) {
    env.storage()
        .instance()
        .set(&DataKey::Threshold, &threshold);
}

/// Overwrites the maximum age, in seconds, a stored price can reach before it is
/// considered stale.
pub(crate) fn store_max_stale_seconds(env: &Env, seconds: u64) {
    env.storage()
        .instance()
        .set(&DataKey::MaxStaleSeconds, &seconds);
}

/// Overwrites the maximum age, in seconds, a signer's submission timestamp can have
/// relative to the ledger time to be accepted.
pub(crate) fn store_max_submission_age(env: &Env, seconds: u64) {
    env.storage()
        .instance()
        .set(&DataKey::MaxSubmissionAgeSeconds, &seconds);
}

/// Overwrites the maximum allowed timestamp skew, in seconds, between signer
/// submissions for the same price update.
pub(crate) fn store_max_relative_skew(env: &Env, seconds: u64) {
    env.storage()
        .instance()
        .set(&DataKey::MaxRelativeSkewSeconds, &seconds);
}

/// Overwrites the configured price resolution.
pub(crate) fn store_resolution(env: &Env, resolution: u32) {
    env.storage()
        .instance()
        .set(&DataKey::Resolution, &resolution);
}

/// Loads the minimum number of signer submissions required to accept a price. Returns 0 if unset.
pub(crate) fn load_threshold(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::Threshold)
        .unwrap_or(0)
}

/// Loads the maximum age, in seconds, a stored price can reach before it is considered stale.
/// Falls back to `DEFAULT_MAX_STALE_SECONDS` if unset.
pub(crate) fn load_max_stale_seconds(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::MaxStaleSeconds)
        .unwrap_or(DEFAULT_MAX_STALE_SECONDS)
}

/// Loads the maximum age, in seconds, a signer's submission timestamp can have relative to the
/// ledger time to be accepted. Falls back to `DEFAULT_MAX_SUBMISSION_AGE_SECONDS` if unset.
pub(crate) fn load_max_submission_age(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::MaxSubmissionAgeSeconds)
        .unwrap_or(DEFAULT_MAX_SUBMISSION_AGE_SECONDS)
}

/// Loads the maximum allowed timestamp skew, in seconds, between signer submissions for the same
/// price update. Falls back to `DEFAULT_MAX_RELATIVE_SKEW_SECONDS` if unset, and clamps the
/// result to the configured maximum submission age.
pub(crate) fn load_max_relative_skew(env: &Env) -> u64 {
    let configured = env
        .storage()
        .instance()
        .get(&DataKey::MaxRelativeSkewSeconds)
        .unwrap_or(DEFAULT_MAX_RELATIVE_SKEW_SECONDS);
    configured.min(load_max_submission_age(env))
}

/// Loads the configured price resolution. Returns 0 if unset.
pub(crate) fn load_resolution(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::Resolution)
        .unwrap_or(0)
}
