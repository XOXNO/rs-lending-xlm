//! Instance and persistent storage: fees, referrals, whitelist, TTL.

use common::constants::{TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};

use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::errors::Error;
use crate::math::checked_add;
use crate::types::{DataKey, ReferralConfig};

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

/// Adds `amount` to the fee bucket balance stored under `key`, persists the result, and adds the
/// same amount to the token's reserved total. Panics with `Error::IntegerOverflow` if either
/// addition overflows `i128`.
pub(crate) fn accumulate_fee(env: &Env, key: DataKey, amount: i128) {
    let token = bucket_token(env, &key);
    credit_bucket(env, key, amount);
    reserve(env, &token, amount);
}

/// Credits both fee buckets a single swap produces for `token` — the admin bucket by
/// `static_amount` and referral `referral_id`'s bucket by `referral_amount` — and reserves the
/// combined amount. A non-positive amount leaves its bucket untouched.
///
/// Writes [`DataKey::ReservedTotal`] once for both buckets. Panics with `Error::IntegerOverflow`
/// if any addition overflows `i128`.
pub(crate) fn accumulate_swap_fees(
    env: &Env,
    token: &Address,
    referral_id: u64,
    static_amount: i128,
    referral_amount: i128,
) {
    let mut reserved = 0_i128;
    if static_amount > 0 {
        credit_bucket(env, DataKey::AdminFee(token.clone()), static_amount);
        reserved = checked_add(env, reserved, static_amount);
    }
    if referral_amount > 0 {
        credit_bucket(
            env,
            DataKey::ReferralFee(referral_id, token.clone()),
            referral_amount,
        );
        reserved = checked_add(env, reserved, referral_amount);
    }
    reserve(env, token, reserved);
}

/// Adds `amount` to the fee bucket balance stored under `key` and persists the result, without
/// touching the token's reserved total. Panics with `Error::IntegerOverflow` on overflow.
///
/// Every call must be paired with a [`reserve`] of the same amount, or `sweep_balance` can
/// transfer fee backing.
fn credit_bucket(env: &Env, key: DataKey, amount: i128) {
    let cur: i128 = env.storage().persistent().get(&key).unwrap_or(0);
    let next = checked_add(env, cur, amount);
    env.storage().persistent().set(&key, &next);
    extend_persistent(env, &key);
}

/// Returns the fee bucket balance stored under `key` and removes the entry from persistent
/// storage when the balance is greater than zero, releasing the same amount from the token's
/// reserved total. Returns 0 if the entry is absent.
pub(crate) fn take_fee_bucket(env: &Env, key: &DataKey) -> i128 {
    let amount: i128 = env.storage().persistent().get(key).unwrap_or(0);
    if amount > 0 {
        env.storage().persistent().remove(key);
        release(env, &bucket_token(env, key), amount);
    }
    amount
}

/// Returns the total fee balance reserved for `token`: the admin fee bucket plus every referral
/// fee bucket, read from the [`DataKey::ReservedTotal`] counter in one lookup.
pub(crate) fn reserved_fee_balance(env: &Env, token: &Address) -> i128 {
    let key = DataKey::ReservedTotal(token.clone());
    let total: i128 = env.storage().persistent().get(&key).unwrap_or(0);
    if total > 0 {
        extend_persistent(env, &key);
    }
    total
}

/// Returns the token a fee bucket is denominated in.
///
/// Inverse of `fees::FeeBucket::key`; a new bucket kind must extend both.
///
/// Panics with [`Error::InternalInvariant`] for a key that is not a fee bucket.
fn bucket_token(env: &Env, key: &DataKey) -> Address {
    match key {
        DataKey::AdminFee(token) | DataKey::ReferralFee(_, token) => token.clone(),
        _ => panic_with_error!(env, Error::InternalInvariant),
    }
}

/// Adds `amount` to `token`'s reserved total. Panics with `Error::IntegerOverflow` on overflow.
///
/// A zero amount writes nothing. Panics with [`Error::InternalInvariant`] on a negative amount,
/// which would let `sweep_balance` transfer fee backing.
fn reserve(env: &Env, token: &Address, amount: i128) {
    if amount < 0 {
        panic_with_error!(env, Error::InternalInvariant);
    }
    if amount == 0 {
        return;
    }
    let key = DataKey::ReservedTotal(token.clone());
    let current: i128 = env.storage().persistent().get(&key).unwrap_or(0);
    let next = checked_add(env, current, amount);
    env.storage().persistent().set(&key, &next);
    extend_persistent(env, &key);
}

/// Subtracts `amount` from `token`'s reserved total, removing the entry once it reaches zero.
///
/// Panics with [`Error::InternalInvariant`] if the total would go negative.
///
/// Callers pass `amount > 0` (a bucket balance that was just removed), so once `current >= amount`
/// the subtraction lands in `0..=current` and cannot overflow.
fn release(env: &Env, token: &Address, amount: i128) {
    let key = DataKey::ReservedTotal(token.clone());
    let current: i128 = env.storage().persistent().get(&key).unwrap_or(0);
    if current < amount {
        panic_with_error!(env, Error::InternalInvariant);
    }
    let next = current - amount;
    if next == 0 {
        env.storage().persistent().remove(&key);
    } else {
        env.storage().persistent().set(&key, &next);
        extend_persistent(env, &key);
    }
}

/// Extends the persistent TTL of `key` using the shared threshold and bump constants.
fn extend_persistent(env: &Env, key: &DataKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}
