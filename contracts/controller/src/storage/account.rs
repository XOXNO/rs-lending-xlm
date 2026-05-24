use super::renew_user_key;
use common::errors::GenericError;
use common::types::{
    Account, AccountMeta, AccountPosition, AccountPositionRaw, AccountPositionType, ControllerKey,
};
use soroban_sdk::{panic_with_error, Address, Env, Map};

fn side_key(account_id: u64, position_type: AccountPositionType) -> ControllerKey {
    match position_type {
        AccountPositionType::Deposit => ControllerKey::SupplyPositions(account_id),
        AccountPositionType::Borrow => ControllerKey::BorrowPositions(account_id),
    }
}

pub(crate) fn account_from_parts(
    meta: AccountMeta,
    supply_positions: Map<Address, AccountPositionRaw>,
    borrow_positions: Map<Address, AccountPositionRaw>,
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

pub(crate) fn get_positions(
    env: &Env,
    account_id: u64,
    position_type: AccountPositionType,
) -> Map<Address, AccountPositionRaw> {
    let key = side_key(account_id, position_type);
    env.storage()
        .persistent()
        .get::<_, Map<Address, AccountPositionRaw>>(&key)
        .unwrap_or_else(|| Map::new(env))
}

pub(crate) fn set_positions(
    env: &Env,
    account_id: u64,
    position_type: AccountPositionType,
    map: &Map<Address, AccountPositionRaw>,
) {
    let key = side_key(account_id, position_type);
    let persistent = env.storage().persistent();
    if map.is_empty() {
        persistent.remove(&key);
    } else {
        persistent.set(&key, map);
    }
    renew_user_account(env, account_id);
}

pub(crate) fn try_get_position(
    env: &Env,
    account_id: u64,
    position_type: AccountPositionType,
    asset: &Address,
) -> Option<AccountPosition> {
    get_positions(env, account_id, position_type)
        .get(asset.clone())
        .map(|raw| AccountPosition::from(&raw))
}

// Lifts each entry to `AccountPosition` so call sites read typed fields
// directly instead of `Ray::from_raw(position.scaled_amount_ray)`.
pub(crate) fn iter_typed_positions(
    map: &Map<Address, AccountPositionRaw>,
) -> impl Iterator<Item = (Address, AccountPosition)> + '_ {
    map.iter()
        .map(|(addr, raw)| (addr, AccountPosition::from(&raw)))
}

pub(crate) fn get_account(env: &Env, account_id: u64) -> Account {
    try_get_account(env, account_id)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AccountNotFound))
}

pub(crate) fn try_get_account(env: &Env, account_id: u64) -> Option<Account> {
    try_get_account_meta(env, account_id).map(|meta| {
        account_from_parts(
            meta,
            get_positions(env, account_id, AccountPositionType::Deposit),
            get_positions(env, account_id, AccountPositionType::Borrow),
        )
    })
}

pub(crate) fn get_account_borrow_only(env: &Env, account_id: u64) -> Account {
    let meta = get_account_meta(env, account_id);
    let borrow_positions = get_positions(env, account_id, AccountPositionType::Borrow);
    account_from_parts(meta, Map::new(env), borrow_positions)
}

pub(crate) fn get_account_supply_only(env: &Env, account_id: u64) -> Account {
    let meta = get_account_meta(env, account_id);
    let supply_positions = get_positions(env, account_id, AccountPositionType::Deposit);
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
