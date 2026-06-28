//! Account storage layout (meta + per-side position maps).
//!
//! Supply and borrow maps are split so one-sided flows touch only one side key.
//! `AccountMeta` stores owner, spoke, and account mode.

use super::renew_user_key;
use common::errors::GenericError;
use controller_interface::types::{
    Account, AccountMeta, AccountPosition, AccountPositionRaw, ControllerKey, DebtPosition,
    DebtPositionRaw, HubAssetKey,
};
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env, Map, Vec};

/// Cap on per-account delegates. The list loads as one persistent entry, so it
/// stays bounded; mirrors the instance-tier approval caps.
const MAX_DELEGATES: u32 = 16;

pub(crate) fn account_from_parts(
    meta: AccountMeta,
    supply_positions: Map<HubAssetKey, AccountPositionRaw>,
    borrow_positions: Map<HubAssetKey, DebtPositionRaw>,
) -> Account {
    Account {
        owner: meta.owner,
        spoke_id: meta.spoke_id,
        mode: meta.mode,
        supply_positions,
        borrow_positions,
    }
}

pub(crate) fn try_get_account_meta(env: &Env, account_id: u64) -> Option<AccountMeta> {
    env.storage()
        .persistent()
        .get::<_, AccountMeta>(&ControllerKey::AccountMeta(account_id))
}

pub(crate) fn get_account_meta(env: &Env, account_id: u64) -> AccountMeta {
    try_get_account_meta(env, account_id)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AccountNotInMarket))
}

pub(crate) fn set_account_meta(env: &Env, account_id: u64, meta: &AccountMeta) {
    let key = ControllerKey::AccountMeta(account_id);
    env.storage().persistent().set(&key, meta);
    renew_user_key(env, &key);
}

pub(crate) fn get_supply_positions(
    env: &Env,
    account_id: u64,
) -> Map<HubAssetKey, AccountPositionRaw> {
    env.storage()
        .persistent()
        .get::<_, Map<HubAssetKey, AccountPositionRaw>>(&ControllerKey::SupplyPositions(account_id))
        .unwrap_or_else(|| Map::new(env))
}

pub(crate) fn get_debt_positions(env: &Env, account_id: u64) -> Map<HubAssetKey, DebtPositionRaw> {
    env.storage()
        .persistent()
        .get::<_, Map<HubAssetKey, DebtPositionRaw>>(&ControllerKey::BorrowPositions(account_id))
        .unwrap_or_else(|| Map::new(env))
}

pub(crate) fn set_supply_positions(
    env: &Env,
    account_id: u64,
    map: &Map<HubAssetKey, AccountPositionRaw>,
) {
    write_side_map(env, &ControllerKey::SupplyPositions(account_id), map);
    renew_user_account(env, account_id);
}

pub(crate) fn set_debt_positions(
    env: &Env,
    account_id: u64,
    map: &Map<HubAssetKey, DebtPositionRaw>,
) {
    write_side_map(env, &ControllerKey::BorrowPositions(account_id), map);
    renew_user_account(env, account_id);
}

fn write_side_map<
    V: soroban_sdk::TryFromVal<Env, soroban_sdk::Val> + soroban_sdk::IntoVal<Env, soroban_sdk::Val>,
>(
    env: &Env,
    key: &ControllerKey,
    map: &Map<HubAssetKey, V>,
) {
    let persistent = env.storage().persistent();
    if map.is_empty() {
        persistent.remove(key);
    } else {
        persistent.set(key, map);
    }
}

pub(crate) fn try_get_supply_position(
    env: &Env,
    account_id: u64,
    hub_asset: &HubAssetKey,
) -> Option<AccountPosition> {
    get_supply_positions(env, account_id)
        .get(hub_asset.clone())
        .map(|raw| AccountPosition::from(&raw))
}

pub(crate) fn try_get_debt_position(
    env: &Env,
    account_id: u64,
    hub_asset: &HubAssetKey,
) -> Option<DebtPosition> {
    get_debt_positions(env, account_id)
        .get(hub_asset.clone())
        .map(|raw| DebtPosition::from(&raw))
}

// Lifts each entry to `AccountPosition` so call sites read typed fields
// instead of `Ray::from(position.scaled_amount_ray)`.
pub(crate) fn iter_typed_positions(
    map: &Map<HubAssetKey, AccountPositionRaw>,
) -> impl Iterator<Item = (HubAssetKey, AccountPosition)> + '_ {
    map.iter()
        .map(|(key, raw)| (key, AccountPosition::from(&raw)))
}

// Debt-side counterpart of `iter_typed_positions`; debt positions carry only
// the scaled share.
pub(crate) fn iter_debt_positions(
    map: &Map<HubAssetKey, DebtPositionRaw>,
) -> impl Iterator<Item = (HubAssetKey, DebtPosition)> + '_ {
    map.iter()
        .map(|(key, raw)| (key, DebtPosition::from(&raw)))
}

pub(crate) fn get_account(env: &Env, account_id: u64) -> Account {
    try_get_account(env, account_id)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AccountNotFound))
}

pub(crate) fn try_get_account(env: &Env, account_id: u64) -> Option<Account> {
    try_get_account_meta(env, account_id).map(|meta| {
        account_from_parts(
            meta,
            get_supply_positions(env, account_id),
            get_debt_positions(env, account_id),
        )
    })
}

pub(crate) fn get_account_borrow_only(env: &Env, account_id: u64) -> Account {
    let meta = get_account_meta(env, account_id);
    let borrow_positions = get_debt_positions(env, account_id);
    account_from_parts(meta, Map::new(env), borrow_positions)
}

/// Opt-in delegate list for an account; empty when none are set.
pub(crate) fn get_delegates(env: &Env, account_id: u64) -> Vec<Address> {
    env.storage()
        .persistent()
        .get(&ControllerKey::Delegates(account_id))
        .unwrap_or_else(|| Vec::new(env))
}

/// Persists the delegate list, removing the entry when it becomes empty so a
/// fully-revoked account leaves no residual storage.
pub(crate) fn set_delegates(env: &Env, account_id: u64, delegates: &Vec<Address>) {
    let key = ControllerKey::Delegates(account_id);
    if delegates.is_empty() {
        env.storage().persistent().remove(&key);
    } else {
        env.storage().persistent().set(&key, delegates);
        renew_user_key(env, &key);
    }
}

/// Adds `delegate` once; re-adding an existing delegate is a no-op. Rejects
/// growth past `MAX_DELEGATES` so the persisted list stays bounded.
pub(crate) fn add_delegate(env: &Env, account_id: u64, delegate: &Address) {
    let mut delegates = get_delegates(env, account_id);
    if delegates.contains(delegate.clone()) {
        return;
    }
    assert_with_error!(
        env,
        delegates.len() < MAX_DELEGATES,
        GenericError::InvalidPositionLimits
    );
    delegates.push_back(delegate.clone());
    set_delegates(env, account_id, &delegates);
}

/// Removes `delegate` if present; absent removal is a no-op.
pub(crate) fn remove_delegate(env: &Env, account_id: u64, delegate: &Address) {
    let delegates = get_delegates(env, account_id);
    if !delegates.contains(delegate.clone()) {
        return;
    }
    let mut next = Vec::new(env);
    for existing in delegates.iter() {
        if existing != *delegate {
            next.push_back(existing);
        }
    }
    set_delegates(env, account_id, &next);
}

pub(crate) fn remove_account_entry(env: &Env, account_id: u64) {
    let persistent = env.storage().persistent();
    persistent.remove(&ControllerKey::AccountMeta(account_id));
    persistent.remove(&ControllerKey::SupplyPositions(account_id));
    persistent.remove(&ControllerKey::BorrowPositions(account_id));
}

// Extends TTL on each existing account key. The `has()` guard is required:
// soroban-sdk 26.x panics on `extend_ttl` against a missing key.
pub(crate) fn renew_user_account(env: &Env, account_id: u64) {
    let persistent = env.storage().persistent();
    let keys = [
        ControllerKey::AccountMeta(account_id),
        ControllerKey::SupplyPositions(account_id),
        ControllerKey::BorrowPositions(account_id),
    ];
    for key in &keys {
        if persistent.has(key) {
            renew_user_key(env, key);
        }
    }
}

#[cfg(test)]
#[path = "../../tests/storage/account.rs"]
mod tests;
