//! Account entries use persistent user storage; successful reads renew user TTL.
//! Metadata and delegate writes renew TTL; position-map writes do not.

use super::protocol::{get_user, renew_user_key, set_user};
use crate::constants::MAX_DELEGATES;
use crate::external::position_nft::nft_try_owner_of_call;
use common::errors::GenericError;
use common::types::{
    Account, AccountMeta, AccountPosition, AccountPositionRaw, ControllerKey, DebtPosition,
    DebtPositionRaw, DelegateGrant, HubAssetKey,
};
use soroban_sdk::{assert_with_error, contracttype, panic_with_error, Address, Env, Map, Vec};

/// Assembles an account from the resolved owner, metadata, and raw position maps.
pub(crate) fn account_from_parts(
    owner: Address,
    meta: AccountMeta,
    supply_positions: Map<HubAssetKey, AccountPositionRaw>,
    borrow_positions: Map<HubAssetKey, DebtPositionRaw>,
) -> Account {
    Account {
        owner,
        spoke_id: meta.spoke_id,
        mode: meta.mode,
        supply_positions,
        borrow_positions,
    }
}

/// Resolves current NFT ownership; returns `None` for an unconfigured NFT,
/// unmintable ID, missing token, or failed lookup. Ownership fails closed.
pub(crate) fn try_account_owner(env: &Env, account_id: u64) -> Option<Address> {
    let nft = super::protocol::try_get_position_nft(env)?;
    nft_try_owner_of_call(env, &nft, account_id)
}

/// Resolves current NFT ownership or fails with `AccountNotFound`.
pub(crate) fn account_owner(env: &Env, account_id: u64) -> Address {
    try_account_owner(env, account_id)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AccountNotFound))
}

/// Returns stored account metadata, or `None` when absent.
pub(crate) fn try_get_account_meta(env: &Env, account_id: u64) -> Option<AccountMeta> {
    get_user(env, &ControllerKey::AccountMeta(account_id))
}

/// Returns account metadata or fails with `AccountNotInMarket`.
pub(crate) fn get_account_meta(env: &Env, account_id: u64) -> AccountMeta {
    try_get_account_meta(env, account_id)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AccountNotInMarket))
}

/// Stores account metadata and renews user TTL.
pub(crate) fn set_account_meta(env: &Env, account_id: u64, meta: &AccountMeta) {
    set_user(env, &ControllerKey::AccountMeta(account_id), meta);
}

/// Returns raw supply positions, defaulting to an empty map.
pub(crate) fn get_supply_positions(
    env: &Env,
    account_id: u64,
) -> Map<HubAssetKey, AccountPositionRaw> {
    get_user(env, &ControllerKey::SupplyPositions(account_id)).unwrap_or_else(|| Map::new(env))
}

/// Returns raw debt positions, defaulting to an empty map.
pub(crate) fn get_debt_positions(env: &Env, account_id: u64) -> Map<HubAssetKey, DebtPositionRaw> {
    get_user(env, &ControllerKey::BorrowPositions(account_id)).unwrap_or_else(|| Map::new(env))
}

/// Stores supply positions without renewing TTL; deletes an empty map.
pub(crate) fn set_supply_positions(
    env: &Env,
    account_id: u64,
    map: &Map<HubAssetKey, AccountPositionRaw>,
) {
    write_side_map(env, &ControllerKey::SupplyPositions(account_id), map);
}

/// Stores debt positions without renewing TTL; deletes an empty map.
pub(crate) fn set_debt_positions(
    env: &Env,
    account_id: u64,
    map: &Map<HubAssetKey, DebtPositionRaw>,
) {
    write_side_map(env, &ControllerKey::BorrowPositions(account_id), map);
}

/// Stores a nonempty position map without renewing TTL; deletes an empty map.
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

/// Returns the typed supply position for this hub asset, or `None`.
pub(crate) fn try_get_supply_position(
    env: &Env,
    account_id: u64,
    hub_asset: &HubAssetKey,
) -> Option<AccountPosition> {
    get_supply_positions(env, account_id)
        .get(hub_asset.clone())
        .map(|raw| AccountPosition::from(&raw))
}

/// Returns the typed debt position for this hub asset, or `None`.
pub(crate) fn try_get_debt_position(
    env: &Env,
    account_id: u64,
    hub_asset: &HubAssetKey,
) -> Option<DebtPosition> {
    get_debt_positions(env, account_id)
        .get(hub_asset.clone())
        .map(|raw| DebtPosition::from(&raw))
}

/// Iterates supply positions with typed fixed-point values.
pub(crate) fn iter_typed_positions(
    map: &Map<HubAssetKey, AccountPositionRaw>,
) -> impl Iterator<Item = (HubAssetKey, AccountPosition)> + '_ {
    map.iter()
        .map(|(key, raw)| (key, AccountPosition::from(&raw)))
}

/// Iterates debt positions with typed fixed-point values.
pub(crate) fn iter_debt_positions(
    map: &Map<HubAssetKey, DebtPositionRaw>,
) -> impl Iterator<Item = (HubAssetKey, DebtPosition)> + '_ {
    map.iter().map(|(key, raw)| (key, DebtPosition::from(&raw)))
}

/// Loads both position maps and current NFT ownership. Missing metadata or
/// unresolved ownership fails with `AccountNotFound`.
pub(crate) fn get_account(env: &Env, account_id: u64) -> Account {
    try_get_account(env, account_id)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AccountNotFound))
}

/// Loads both position maps and current NFT ownership; returns `None` when
/// metadata is absent or ownership cannot be resolved.
pub(crate) fn try_get_account(env: &Env, account_id: u64) -> Option<Account> {
    let meta = try_get_account_meta(env, account_id)?;
    let owner = try_account_owner(env, account_id)?;
    Some(account_from_parts(
        owner,
        meta,
        get_supply_positions(env, account_id),
        get_debt_positions(env, account_id),
    ))
}

/// Loads metadata, current owner, and debt; leaves supply deliberately unloaded.
/// Missing metadata raises `AccountNotInMarket`; unresolved ownership raises
/// `AccountNotFound`. The empty supply map does not prove supply is absent.
pub(crate) fn get_account_borrow_only(env: &Env, account_id: u64) -> Account {
    let meta = get_account_meta(env, account_id);
    let owner = account_owner(env, account_id);
    let borrow_positions = get_debt_positions(env, account_id);
    account_from_parts(owner, meta, Map::new(env), borrow_positions)
}

/// Returns grants stamped by `owner`, or an empty list. Ownership changes
/// invalidate a previous owner's grants without deleting them.
pub(crate) fn get_delegates(env: &Env, account_id: u64, owner: &Address) -> Vec<Address> {
    get_user::<DelegateGrant>(env, &ControllerKey::Delegates(account_id))
        .filter(|grant| grant.granted_by == *owner)
        .map(|grant| grant.delegates)
        .unwrap_or_else(|| Vec::new(env))
}

/// Stores delegates stamped by the granting owner and renews user TTL;
/// deletes the entry when the list is empty.
fn set_delegates(env: &Env, account_id: u64, owner: &Address, delegates: &Vec<Address>) {
    let key = ControllerKey::Delegates(account_id);
    if delegates.is_empty() {
        env.storage().persistent().remove(&key);
    } else {
        set_user(
            env,
            &key,
            &DelegateGrant {
                granted_by: owner.clone(),
                delegates: delegates.clone(),
            },
        );
    }
}

/// Adds a delegate, enforcing `MAX_DELEGATES`; returns false for duplicates.
/// Overwrites stale grants with a list stamped by the current owner.
pub(crate) fn add_delegate(
    env: &Env,
    account_id: u64,
    owner: &Address,
    delegate: &Address,
) -> bool {
    let mut delegates = get_delegates(env, account_id, owner);
    if delegates.contains(delegate) {
        return false;
    }
    assert_with_error!(
        env,
        delegates.len() < MAX_DELEGATES,
        GenericError::RegistryCapReached
    );
    delegates.push_back(delegate.clone());
    set_delegates(env, account_id, owner, &delegates);
    true
}

/// Removes a live delegate and reports whether it changed the list.
/// Deletes stale grants but returns false, preventing those grants from
/// reactivating if the NFT returns to their original owner.
pub(crate) fn remove_delegate(
    env: &Env,
    account_id: u64,
    owner: &Address,
    delegate: &Address,
) -> bool {
    let key = ControllerKey::Delegates(account_id);
    let Some(grant) = get_user::<DelegateGrant>(env, &key) else {
        return false;
    };
    if grant.granted_by != *owner {
        env.storage().persistent().remove(&key);
        return false;
    }
    let mut delegates = grant.delegates;
    let Some(index) = delegates.first_index_of(delegate) else {
        return false;
    };
    delegates.remove(index);
    set_delegates(env, account_id, owner, &delegates);
    true
}

/// Deletes metadata, both position maps, and delegates. Does not burn the NFT.
pub(crate) fn remove_account_entry(env: &Env, account_id: u64) {
    let persistent = env.storage().persistent();
    persistent.remove(&ControllerKey::AccountMeta(account_id));
    persistent.remove(&ControllerKey::SupplyPositions(account_id));
    persistent.remove(&ControllerKey::BorrowPositions(account_id));
    persistent.remove(&ControllerKey::Delegates(account_id));
}

/// Renews user TTL for each existing account entry; does not renew the NFT.
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

#[contracttype]
#[derive(Clone, Debug)]
enum SessionKey {
    FlashLoanOngoing,
}

/// Returns the temporary flash-loan flag, defaulting to false.
pub(crate) fn is_flash_loan_ongoing(env: &Env) -> bool {
    env.storage()
        .temporary()
        .get(&SessionKey::FlashLoanOngoing)
        .unwrap_or(false)
}

/// Sets the temporary flash-loan flag, or removes it when clearing.
pub(crate) fn set_flash_loan_ongoing(env: &Env, ongoing: bool) {
    if ongoing {
        env.storage()
            .temporary()
            .set(&SessionKey::FlashLoanOngoing, &true);
    } else {
        env.storage()
            .temporary()
            .remove(&SessionKey::FlashLoanOngoing);
    }
}

/// Runs `f` with the flash-loan flag set; preserves an already-active outer guard.
pub(crate) fn with_flash_guard<T>(env: &Env, f: impl FnOnce() -> T) -> T {
    let prev = is_flash_loan_ongoing(env);
    set_flash_loan_ongoing(env, true);
    let out = f();
    if !prev {
        set_flash_loan_ongoing(env, false);
    }
    out
}
