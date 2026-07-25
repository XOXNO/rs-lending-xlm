//! Instance-storage configuration reads. Every getter has a safe default so
//! a partially-configured contract reads as inert rather than trapping.

use soroban_sdk::{Address, Env, Vec};

use crate::storage::{
    DataKey, DEFAULT_MAX_RELATIVE_SKEW_SECONDS, DEFAULT_MAX_STALE_SECONDS,
    DEFAULT_MAX_SUBMISSION_AGE_SECONDS,
};

/// Registered signer set; empty before the constructor runs.
pub(crate) fn load_signers(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&DataKey::Signers)
        .unwrap_or_else(|| Vec::new(env))
}

/// Submissions required to publish an aggregate; `0` until configured.
pub(crate) fn load_threshold(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::Threshold)
        .unwrap_or(0)
}

/// Age past which a published aggregate stops being readable.
pub(crate) fn load_max_stale_seconds(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::MaxStaleSeconds)
        .unwrap_or(DEFAULT_MAX_STALE_SECONDS)
}

/// Absolute inclusion window: submissions older than this are rejected outright.
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

/// History bucket width in seconds; samples inside one bucket overwrite in place.
pub(crate) fn load_resolution(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::Resolution)
        .unwrap_or(0)
}
