use super::renew_user_key;
use common::errors::GenericError;
use common::types::{
    Account, AccountMeta, AccountPosition, AccountPositionRaw, ControllerKey, DebtPosition,
    DebtPositionRaw,
};
use soroban_sdk::{panic_with_error, Address, Env, Map};

pub(crate) fn account_from_parts(
    meta: AccountMeta,
    supply_positions: Map<Address, AccountPositionRaw>,
    borrow_positions: Map<Address, DebtPositionRaw>,
) -> Account {
    Account {
        owner: meta.owner,
        is_isolated: meta.is_isolated,
        e_mode_category_id: meta.e_mode_category_id,
        mode: meta.mode,
        isolated_asset: meta.isolated_asset,
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

pub(crate) fn get_supply_positions(env: &Env, account_id: u64) -> Map<Address, AccountPositionRaw> {
    env.storage()
        .persistent()
        .get::<_, Map<Address, AccountPositionRaw>>(&ControllerKey::SupplyPositions(account_id))
        .unwrap_or_else(|| Map::new(env))
}

pub(crate) fn get_debt_positions(env: &Env, account_id: u64) -> Map<Address, DebtPositionRaw> {
    env.storage()
        .persistent()
        .get::<_, Map<Address, DebtPositionRaw>>(&ControllerKey::BorrowPositions(account_id))
        .unwrap_or_else(|| Map::new(env))
}

pub(crate) fn set_supply_positions(
    env: &Env,
    account_id: u64,
    map: &Map<Address, AccountPositionRaw>,
) {
    write_side_map(env, &ControllerKey::SupplyPositions(account_id), map);
    renew_user_account(env, account_id);
}

pub(crate) fn set_debt_positions(env: &Env, account_id: u64, map: &Map<Address, DebtPositionRaw>) {
    write_side_map(env, &ControllerKey::BorrowPositions(account_id), map);
    renew_user_account(env, account_id);
}

fn write_side_map<
    V: soroban_sdk::TryFromVal<Env, soroban_sdk::Val> + soroban_sdk::IntoVal<Env, soroban_sdk::Val>,
>(
    env: &Env,
    key: &ControllerKey,
    map: &Map<Address, V>,
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
    asset: &Address,
) -> Option<AccountPosition> {
    get_supply_positions(env, account_id)
        .get(asset.clone())
        .map(|raw| AccountPosition::from(&raw))
}

pub(crate) fn try_get_debt_position(
    env: &Env,
    account_id: u64,
    asset: &Address,
) -> Option<DebtPosition> {
    get_debt_positions(env, account_id)
        .get(asset.clone())
        .map(|raw| DebtPosition::from(&raw))
}

// Lifts each entry to `AccountPosition` so call sites read typed fields
// directly instead of `Ray::from_raw(position.scaled_amount_ray)`.
pub(crate) fn iter_typed_positions(
    map: &Map<Address, AccountPositionRaw>,
) -> impl Iterator<Item = (Address, AccountPosition)> + '_ {
    map.iter()
        .map(|(addr, raw)| (addr, AccountPosition::from(&raw)))
}

// Debt-side counterpart of `iter_typed_positions`. Debt positions carry only
// the scaled share.
pub(crate) fn iter_debt_positions(
    map: &Map<Address, DebtPositionRaw>,
) -> impl Iterator<Item = (Address, DebtPosition)> + '_ {
    map.iter()
        .map(|(addr, raw)| (addr, DebtPosition::from(&raw)))
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

pub(crate) fn get_account_supply_only(env: &Env, account_id: u64) -> Account {
    let meta = get_account_meta(env, account_id);
    let supply_positions = get_supply_positions(env, account_id);
    account_from_parts(meta, supply_positions, Map::new(env))
}

pub(crate) fn remove_account_entry(env: &Env, account_id: u64) {
    let persistent = env.storage().persistent();
    persistent.remove(&ControllerKey::AccountMeta(account_id));
    persistent.remove(&ControllerKey::SupplyPositions(account_id));
    persistent.remove(&ControllerKey::BorrowPositions(account_id));
}

// Extends TTL on every existing account key. The `has()` guard is required —
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
