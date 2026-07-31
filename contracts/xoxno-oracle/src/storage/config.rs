use soroban_sdk::{Address, Env, Vec};

use crate::storage::{
    DataKey, DEFAULT_MAX_RELATIVE_SKEW_SECONDS, DEFAULT_MAX_STALE_SECONDS,
    DEFAULT_MAX_SUBMISSION_AGE_SECONDS,
};

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
