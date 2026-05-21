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

fn read_side_map(
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

fn write_side_map(
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
        renew_user_key(env, &key);
    }
    // Renew keys to prevent archiving.
    let meta_key = account_meta_key(account_id);
    if persistent.has(&meta_key) {
        renew_user_key(env, &meta_key);
    }
    let other_type = match position_type {
        AccountPositionType::Deposit => AccountPositionType::Borrow,
        AccountPositionType::Borrow => AccountPositionType::Deposit,
    };
    let other_key = side_key(account_id, other_type);
    if persistent.has(&other_key) {
        renew_user_key(env, &other_key);
    }
}

fn account_meta_key(account_id: u64) -> ControllerKey {
    ControllerKey::AccountMeta(account_id)
}

pub fn account_from_parts(
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

fn account_from_meta(env: &Env, account_id: u64, meta: &AccountMeta) -> Account {
    Account {
        owner: meta.owner.clone(),
        is_isolated: meta.is_isolated,
        e_mode_category_id: meta.e_mode_category_id,
        mode: meta.mode,
        isolated_asset: meta.isolated_asset.clone(),
        supply_positions: read_side_map(env, account_id, AccountPositionType::Deposit),
        borrow_positions: read_side_map(env, account_id, AccountPositionType::Borrow),
    }
}

pub fn try_get_account_meta(env: &Env, account_id: u64) -> Option<AccountMeta> {
    env.storage()
        .persistent()
        .get::<_, AccountMeta>(&account_meta_key(account_id))
}

pub fn get_account_meta(env: &Env, account_id: u64) -> AccountMeta {
    try_get_account_meta(env, account_id)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AccountNotInMarket))
}

// Updates meta and extends TTL.
pub fn set_account_meta(env: &Env, account_id: u64, meta: &AccountMeta) {
    let key = account_meta_key(account_id);
    let persistent = env.storage().persistent();
    if persistent.get::<_, AccountMeta>(&key).as_ref() != Some(meta) {
        persistent.set(&key, meta);
    }
    renew_user_key(env, &key);
}

pub fn try_get_position(
    env: &Env,
    account_id: u64,
    position_type: AccountPositionType,
    asset: &Address,
) -> Option<AccountPosition> {
    let map = read_side_map(env, account_id, position_type);
    map.get(asset.clone()).map(|raw| AccountPosition::from(&raw))
}

pub fn get_supply_positions(env: &Env, account_id: u64) -> Map<Address, AccountPositionRaw> {
    read_side_map(env, account_id, AccountPositionType::Deposit)
}

pub fn get_borrow_positions(env: &Env, account_id: u64) -> Map<Address, AccountPositionRaw> {
    read_side_map(env, account_id, AccountPositionType::Borrow)
}

pub fn set_supply_positions(env: &Env, account_id: u64, map: &Map<Address, AccountPositionRaw>) {
    write_side_map(env, account_id, AccountPositionType::Deposit, map);
}

pub fn set_borrow_positions(env: &Env, account_id: u64, map: &Map<Address, AccountPositionRaw>) {
    write_side_map(env, account_id, AccountPositionType::Borrow, map);
}

// Typed iterator over a position map. Lifts each entry from `AccountPositionRaw`
// to `AccountPosition` so call sites can read `position.scaled_amount` etc.
// directly instead of `Ray::from_raw(position.scaled_amount_ray)`.
pub fn iter_typed_positions(
    map: &Map<Address, AccountPositionRaw>,
) -> impl Iterator<Item = (Address, AccountPosition)> + '_ {
    map.iter().map(|(addr, raw)| (addr, AccountPosition::from(&raw)))
}

pub fn get_account(env: &Env, account_id: u64) -> Account {
    try_get_account(env, account_id)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AccountNotFound))
}

pub fn try_get_account(env: &Env, account_id: u64) -> Option<Account> {
    try_get_account_meta(env, account_id).map(|meta| account_from_meta(env, account_id, &meta))
}

// Removes account and positions.
pub fn remove_account_entry(env: &Env, account_id: u64) {
    let persistent = env.storage().persistent();
    persistent.remove(&account_meta_key(account_id));
    persistent.remove(&side_key(account_id, AccountPositionType::Deposit));
    persistent.remove(&side_key(account_id, AccountPositionType::Borrow));
}

// Renews account storage TTL.
pub fn renew_user_account(env: &Env, account_id: u64) {
    let persistent = env.storage().persistent();
    let meta_key = account_meta_key(account_id);
    if persistent.has(&meta_key) {
        renew_user_key(env, &meta_key);
    }
    let supply_key = side_key(account_id, AccountPositionType::Deposit);
    if persistent.has(&supply_key) {
        renew_user_key(env, &supply_key);
    }
    let borrow_key = side_key(account_id, AccountPositionType::Borrow);
    if persistent.has(&borrow_key) {
        renew_user_key(env, &borrow_key);
    }
}
