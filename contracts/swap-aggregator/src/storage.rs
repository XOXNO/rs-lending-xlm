//! Instance and persistent storage: fees, referrals, whitelist, TTL.

use common::constants::{TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};

use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::errors::Error;
use crate::math::checked_add;
use crate::types::{DataKey, ReferralConfig};

/// Extend instance TTL on hot write paths.
pub(crate) fn renew_instance(env: &Env) {
    common::ttl::renew_instance(env);
}

/// Static protocol fee in bps (instance).
pub(crate) fn static_fee_bps(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::StaticFeeBps)
        .unwrap_or(0)
}

/// Persist static protocol fee.
pub(crate) fn set_static_fee_bps(env: &Env, fee_bps: u32) {
    env.storage()
        .instance()
        .set(&DataKey::StaticFeeBps, &fee_bps);
}

/// Fee-whitelist tokens, or empty vec if unset.
pub(crate) fn load_whitelist(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&DataKey::WhitelistedTokens)
        .unwrap_or_else(|| Vec::new(env))
}

/// Replace the fee-whitelist list.
pub(crate) fn set_whitelist(env: &Env, list: &Vec<Address>) {
    env.storage()
        .instance()
        .set(&DataKey::WhitelistedTokens, list);
}

/// Highest referral id issued (0 if none).
pub(crate) fn referral_counter(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::ReferralCounter)
        .unwrap_or(0)
}

/// Persist referral counter.
pub(crate) fn set_referral_counter(env: &Env, id: u64) {
    env.storage().instance().set(&DataKey::ReferralCounter, &id);
}

/// Load referral or panic [`Error::ReferralNotFound`]. Extends TTL.
pub(crate) fn load_referral(env: &Env, id: u64) -> ReferralConfig {
    let key = DataKey::Referral(id);
    let v: ReferralConfig = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic_with_error!(env, Error::ReferralNotFound));
    extend_persistent(env, &key);
    v
}

/// Load referral if present; extends TTL when found.
pub(crate) fn try_load_referral(env: &Env, id: u64) -> Option<ReferralConfig> {
    let key = DataKey::Referral(id);
    let v: Option<ReferralConfig> = env.storage().persistent().get(&key);
    if v.is_some() {
        extend_persistent(env, &key);
    }
    v
}

/// Write referral config under `id`.
pub(crate) fn set_referral(env: &Env, id: u64, cfg: &ReferralConfig) {
    env.storage().persistent().set(&DataKey::Referral(id), cfg);
}

/// Fee bucket balance for `key`; extends TTL when non-zero.
pub(crate) fn fee_balance(env: &Env, key: &DataKey) -> i128 {
    let v: Option<i128> = env.storage().persistent().get(key);
    if v.is_some() {
        extend_persistent(env, key);
    }
    v.unwrap_or(0)
}

/// Add `amount` into fee bucket `key`.
pub(crate) fn accumulate_fee(env: &Env, key: DataKey, amount: i128) {
    let cur: i128 = env.storage().persistent().get(&key).unwrap_or(0);
    let next = checked_add(env, cur, amount);
    env.storage().persistent().set(&key, &next);
}

/// Read and clear a fee bucket. Returns 0 if empty.
pub(crate) fn take_fee_bucket(env: &Env, key: &DataKey) -> i128 {
    let amount: i128 = env.storage().persistent().get(key).unwrap_or(0);
    if amount > 0 {
        env.storage().persistent().remove(key);
    }
    amount
}

/// Admin + all referral fee buckets for `token` (sweep reserve). Extends TTL on hits.
pub(crate) fn reserved_fee_balance(env: &Env, token: &Address) -> i128 {
    let admin_key = DataKey::AdminFee(token.clone());
    let mut total: i128 = env.storage().persistent().get(&admin_key).unwrap_or(0);
    if total > 0 {
        extend_persistent(env, &admin_key);
    }

    let counter = referral_counter(env);
    for id in 1..=counter {
        let key = DataKey::ReferralFee(id, token.clone());
        let amount: i128 = env.storage().persistent().get(&key).unwrap_or(0);
        if amount > 0 {
            extend_persistent(env, &key);
        }
        total = checked_add(env, total, amount);
    }
    total
}

fn extend_persistent(env: &Env, key: &DataKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}
