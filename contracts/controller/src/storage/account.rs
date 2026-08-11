//! Storage accessors for per-account controller state: account metadata,
//! supply and debt position maps, and delegate lists.

use crate::constants::MAX_DELEGATES;
use crate::storage::{get_user, renew_user_key, set_user};
use common::errors::GenericError;
use common::types::{
    Account, AccountMeta, AccountPosition, AccountPositionRaw, ControllerKey, DebtPosition,
    DebtPositionRaw, HubAssetKey,
};
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env, Map, Vec};

/// Assembles an `Account` from its metadata and raw supply/debt position maps.
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

/// Reads account metadata for `account_id`. Returns `None` if the account does not exist.
pub(crate) fn try_get_account_meta(env: &Env, account_id: u64) -> Option<AccountMeta> {
    get_user(env, &ControllerKey::AccountMeta(account_id))
}

/// Reads account metadata for `account_id`. Panics with `AccountNotInMarket`
/// if the account does not exist.
pub(crate) fn get_account_meta(env: &Env, account_id: u64) -> AccountMeta {
    try_get_account_meta(env, account_id)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AccountNotInMarket))
}

/// Writes account metadata for `account_id`.
pub(crate) fn set_account_meta(env: &Env, account_id: u64, meta: &AccountMeta) {
    set_user(env, &ControllerKey::AccountMeta(account_id), meta);
}

/// Reads the raw supply positions map for `account_id`. Returns an empty map if none is stored.
pub(crate) fn get_supply_positions(
    env: &Env,
    account_id: u64,
) -> Map<HubAssetKey, AccountPositionRaw> {
    get_user(env, &ControllerKey::SupplyPositions(account_id)).unwrap_or_else(|| Map::new(env))
}

/// Reads the raw debt (borrow) positions map for `account_id`. Returns an
/// empty map if none is stored.
pub(crate) fn get_debt_positions(env: &Env, account_id: u64) -> Map<HubAssetKey, DebtPositionRaw> {
    get_user(env, &ControllerKey::BorrowPositions(account_id)).unwrap_or_else(|| Map::new(env))
}

/// Writes supply positions. Empty maps remove the storage key.
///
/// Does not renew account TTL. Callers that mutate positions and must keep the
/// account live should call `renew_user_account` after all side writes (see
/// `positions::persist_account_positions`).
pub(crate) fn set_supply_positions(
    env: &Env,
    account_id: u64,
    map: &Map<HubAssetKey, AccountPositionRaw>,
) {
    write_side_map(env, &ControllerKey::SupplyPositions(account_id), map);
}

/// Writes debt positions. Empty maps remove the storage key.
///
/// Does not renew account TTL. Callers that mutate positions and must keep the
/// account live should call `renew_user_account` after all side writes (see
/// `positions::persist_account_positions`).
pub(crate) fn set_debt_positions(
    env: &Env,
    account_id: u64,
    map: &Map<HubAssetKey, DebtPositionRaw>,
) {
    write_side_map(env, &ControllerKey::BorrowPositions(account_id), map);
}

/// Writes `map` under `key`. Removes the storage key instead if `map` is empty.
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

/// Reads and decodes the typed supply position for `hub_asset` from the account's supply
/// positions map. Returns `None` if the account has no position in that asset.
pub(crate) fn try_get_supply_position(
    env: &Env,
    account_id: u64,
    hub_asset: &HubAssetKey,
) -> Option<AccountPosition> {
    get_supply_positions(env, account_id)
        .get(hub_asset.clone())
        .map(|raw| AccountPosition::from(&raw))
}

/// Reads and decodes the typed debt position for `hub_asset` from the account's debt
/// positions map. Returns `None` if the account has no position in that asset.
pub(crate) fn try_get_debt_position(
    env: &Env,
    account_id: u64,
    hub_asset: &HubAssetKey,
) -> Option<DebtPosition> {
    get_debt_positions(env, account_id)
        .get(hub_asset.clone())
        .map(|raw| DebtPosition::from(&raw))
}

/// Returns an iterator that decodes each raw entry of `map` into a typed `AccountPosition`.
pub(crate) fn iter_typed_positions(
    map: &Map<HubAssetKey, AccountPositionRaw>,
) -> impl Iterator<Item = (HubAssetKey, AccountPosition)> + '_ {
    map.iter()
        .map(|(key, raw)| (key, AccountPosition::from(&raw)))
}

/// Returns an iterator that decodes each raw entry of `map` into a typed `DebtPosition`.
pub(crate) fn iter_debt_positions(
    map: &Map<HubAssetKey, DebtPositionRaw>,
) -> impl Iterator<Item = (HubAssetKey, DebtPosition)> + '_ {
    map.iter().map(|(key, raw)| (key, DebtPosition::from(&raw)))
}

/// Reads the full account (metadata, supply positions, debt positions) for `account_id`.
/// Panics with `AccountNotFound` if the account does not exist.
pub(crate) fn get_account(env: &Env, account_id: u64) -> Account {
    try_get_account(env, account_id)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AccountNotFound))
}

/// Reads the full account (metadata, supply positions, debt positions) for `account_id`.
/// Returns `None` if the account's metadata does not exist.
pub(crate) fn try_get_account(env: &Env, account_id: u64) -> Option<Account> {
    try_get_account_meta(env, account_id).map(|meta| {
        account_from_parts(
            meta,
            get_supply_positions(env, account_id),
            get_debt_positions(env, account_id),
        )
    })
}

/// Reads the account with its debt positions but an empty supply positions map.
/// Panics with `AccountNotInMarket` if the account's metadata does not exist.
pub(crate) fn get_account_borrow_only(env: &Env, account_id: u64) -> Account {
    let meta = get_account_meta(env, account_id);
    let borrow_positions = get_debt_positions(env, account_id);
    account_from_parts(meta, Map::new(env), borrow_positions)
}

/// Reads the delegate addresses for `account_id`. Returns an empty vector if none is stored.
pub(crate) fn get_delegates(env: &Env, account_id: u64) -> Vec<Address> {
    get_user(env, &ControllerKey::Delegates(account_id)).unwrap_or_else(|| Vec::new(env))
}

/// Writes the delegate addresses for `account_id`. Removes the storage key instead if
/// `delegates` is empty.
pub(crate) fn set_delegates(env: &Env, account_id: u64, delegates: &Vec<Address>) {
    let key = ControllerKey::Delegates(account_id);
    if delegates.is_empty() {
        env.storage().persistent().remove(&key);
    } else {
        set_user(env, &key, delegates);
    }
}

/// Adds `delegate` to the account's delegate list if not already present. Returns `false`
/// without modifying storage if the delegate is already present. Panics with
/// `RegistryCapReached` if the account already has `MAX_DELEGATES` delegates.
pub(crate) fn add_delegate(env: &Env, account_id: u64, delegate: &Address) -> bool {
    let mut delegates = get_delegates(env, account_id);
    if delegates.contains(delegate) {
        return false;
    }
    assert_with_error!(
        env,
        delegates.len() < MAX_DELEGATES,
        GenericError::RegistryCapReached
    );
    delegates.push_back(delegate.clone());
    set_delegates(env, account_id, &delegates);
    true
}

/// Removes `delegate` from the account's delegate list. Returns `false` without modifying
/// storage if the delegate is not present.
pub(crate) fn remove_delegate(env: &Env, account_id: u64, delegate: &Address) -> bool {
    let mut delegates = get_delegates(env, account_id);
    let Some(index) = delegates.first_index_of(delegate) else {
        return false;
    };
    delegates.remove(index);
    set_delegates(env, account_id, &delegates);
    true
}

/// Removes all storage entries for `account_id`: metadata, supply positions, debt
/// positions, and delegates.
pub(crate) fn remove_account_entry(env: &Env, account_id: u64) {
    let persistent = env.storage().persistent();
    persistent.remove(&ControllerKey::AccountMeta(account_id));
    persistent.remove(&ControllerKey::SupplyPositions(account_id));
    persistent.remove(&ControllerKey::BorrowPositions(account_id));
    persistent.remove(&ControllerKey::Delegates(account_id));
}

/// Co-renews every live user-account key (meta, supply, debt, delegates).
pub(crate) fn renew_user_account(env: &Env, account_id: u64) {
    let persistent = env.storage().persistent();
    let keys = [
        ControllerKey::AccountMeta(account_id),
        ControllerKey::SupplyPositions(account_id),
        ControllerKey::BorrowPositions(account_id),
        ControllerKey::Delegates(account_id),
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
