//! Instance and persistent storage: fees, referrals, whitelist, TTL.

use common::constants::{TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};

use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::errors::Error;
use crate::math::checked_add;
use crate::types::{DataKey, ReferralConfig};

/// Extends the contract instance's storage TTL.
pub(crate) fn renew_instance(env: &Env) {
    common::ttl::renew_instance(env);
}

/// Returns the static protocol fee in basis points from instance storage, or 0 if unset.
pub(crate) fn static_fee_bps(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::StaticFeeBps)
        .unwrap_or(0)
}

/// Persists the static protocol fee in basis points to instance storage.
pub(crate) fn set_static_fee_bps(env: &Env, fee_bps: u32) {
    env.storage()
        .instance()
        .set(&DataKey::StaticFeeBps, &fee_bps);
}

/// Returns the fee-whitelisted token list from instance storage, or an empty vector if unset.
pub(crate) fn load_whitelist(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&DataKey::WhitelistedTokens)
        .unwrap_or_else(|| Vec::new(env))
}

/// Replaces the stored fee-whitelisted token list.
pub(crate) fn set_whitelist(env: &Env, list: &Vec<Address>) {
    env.storage()
        .instance()
        .set(&DataKey::WhitelistedTokens, list);
}

/// Returns the highest referral id issued, or 0 if none exist.
pub(crate) fn referral_counter(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::ReferralCounter)
        .unwrap_or(0)
}

/// Persists the referral id counter.
pub(crate) fn set_referral_counter(env: &Env, id: u64) {
    env.storage().instance().set(&DataKey::ReferralCounter, &id);
}

/// Loads the referral config for `id` from persistent storage and extends its TTL. Panics with
/// [`Error::ReferralNotFound`] if no referral exists for `id`.
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

/// Loads the referral config for `id` from persistent storage if present, extending its TTL
/// when found. Returns `None` if no referral exists for `id`.
pub(crate) fn try_load_referral(env: &Env, id: u64) -> Option<ReferralConfig> {
    let key = DataKey::Referral(id);
    let v: Option<ReferralConfig> = env.storage().persistent().get(&key);
    if v.is_some() {
        extend_persistent(env, &key);
    }
    v
}

/// Writes the referral config for `id` to persistent storage.
pub(crate) fn set_referral(env: &Env, id: u64, cfg: &ReferralConfig) {
    env.storage().persistent().set(&DataKey::Referral(id), cfg);
}

/// Returns the fee bucket balance stored under `key`, extending its persistent TTL when the
/// entry is present. Returns 0 if the entry is absent.
pub(crate) fn fee_balance(env: &Env, key: &DataKey) -> i128 {
    let v: Option<i128> = env.storage().persistent().get(key);
    if v.is_some() {
        extend_persistent(env, key);
    }
    v.unwrap_or(0)
}

/// Adds `amount` to the fee bucket balance stored under `key` and persists the result. Panics
/// with `Error::IntegerOverflow` if the addition overflows `i128`.
pub(crate) fn accumulate_fee(env: &Env, key: DataKey, amount: i128) {
    let cur: i128 = env.storage().persistent().get(&key).unwrap_or(0);
    let next = checked_add(env, cur, amount);
    env.storage().persistent().set(&key, &next);
}

/// Returns the fee bucket balance stored under `key` and removes the entry from persistent
/// storage when the balance is greater than zero. Returns 0 if the entry is absent.
pub(crate) fn take_fee_bucket(env: &Env, key: &DataKey) -> i128 {
    let amount: i128 = env.storage().persistent().get(key).unwrap_or(0);
    if amount > 0 {
        env.storage().persistent().remove(key);
    }
    amount
}

/// Returns the total fee balance reserved for `token`: the admin fee bucket plus every
/// referral fee bucket up to the current referral counter. Extends the persistent TTL of each
/// non-zero bucket encountered.
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

/// Extends the persistent TTL of `key` using the shared threshold and bump constants.
fn extend_persistent(env: &Env, key: &DataKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}
