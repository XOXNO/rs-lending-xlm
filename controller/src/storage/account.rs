use super::bump_user;
use common::errors::GenericError;
use common::types::{
    Account, AccountMeta, AccountPosition, ControllerKey, POSITION_TYPE_BORROW,
    POSITION_TYPE_DEPOSIT,
};
use soroban_sdk::{panic_with_error, Address, Env, Map};

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn side_key(account_id: u64, position_type: u32) -> ControllerKey {
    if position_type == POSITION_TYPE_DEPOSIT {
        ControllerKey::SupplyPositions(account_id)
    } else {
        ControllerKey::BorrowPositions(account_id)
    }
}

fn read_side_map(
    env: &Env,
    account_id: u64,
    position_type: u32,
) -> Map<Address, AccountPosition> {
    let key = side_key(account_id, position_type);
    env.storage()
        .persistent()
        .get::<_, Map<Address, AccountPosition>>(&key)
        .unwrap_or_else(|| Map::new(env))
}

fn write_side_map(
    env: &Env,
    account_id: u64,
    position_type: u32,
    map: &Map<Address, AccountPosition>,
) {
    let key = side_key(account_id, position_type);
    let persistent = env.storage().persistent();
    if map.is_empty() {
        persistent.remove(&key);
    } else {
        persistent.set(&key, map);
        bump_user(env, &key);
    }
    // Any side write keeps AccountMeta alive at least as long as the most
    // recent user-side activity; without this the meta key could expire
    // independently of live position data and orphan the account.
    let meta_key = account_meta_key(account_id);
    if persistent.has(&meta_key) {
        bump_user(env, &meta_key);
    }
}

fn account_meta_key(account_id: u64) -> ControllerKey {
    ControllerKey::AccountMeta(account_id)
}

fn meta_from_account(account: &Account) -> AccountMeta {
    AccountMeta {
        owner: account.owner.clone(),
        is_isolated: account.is_isolated,
        e_mode_category_id: account.e_mode_category_id,
        mode: account.mode,
        isolated_asset: account.isolated_asset.clone(),
    }
}

/// Builds an `Account` from a meta and pre-loaded side maps. Either side
/// may be empty when the caller only loaded one side and expects inner
/// helpers to consume only that side; helpers that touch both sides at a
/// math gate must load the other side explicitly.
pub fn account_from_parts(
    meta: AccountMeta,
    supply_positions: Map<Address, AccountPosition>,
    borrow_positions: Map<Address, AccountPosition>,
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
        supply_positions: read_side_map(env, account_id, POSITION_TYPE_DEPOSIT),
        borrow_positions: read_side_map(env, account_id, POSITION_TYPE_BORROW),
    }
}

// ---------------------------------------------------------------------------
// AccountMeta API
// ---------------------------------------------------------------------------

pub fn try_get_account_meta(env: &Env, account_id: u64) -> Option<AccountMeta> {
    env.storage()
        .persistent()
        .get::<_, AccountMeta>(&account_meta_key(account_id))
}

pub fn get_account_meta(env: &Env, account_id: u64) -> AccountMeta {
    try_get_account_meta(env, account_id)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AccountNotInMarket))
}

/// Writes meta if it actually changed; always extends meta TTL so the
/// account stays alive even on no-op upserts.
pub fn set_account_meta(env: &Env, account_id: u64, meta: &AccountMeta) {
    let key = account_meta_key(account_id);
    let persistent = env.storage().persistent();
    if persistent.get::<_, AccountMeta>(&key).as_ref() != Some(meta) {
        persistent.set(&key, meta);
    }
    bump_user(env, &key);
}

// ---------------------------------------------------------------------------
// Per-position atomic API
// ---------------------------------------------------------------------------

pub fn try_get_position(
    env: &Env,
    account_id: u64,
    position_type: u32,
    asset: &Address,
) -> Option<AccountPosition> {
    let map = read_side_map(env, account_id, position_type);
    map.get(asset.clone())
}

// ---------------------------------------------------------------------------
// Side enumeration (single storage read)
// ---------------------------------------------------------------------------

pub fn get_supply_positions(env: &Env, account_id: u64) -> Map<Address, AccountPosition> {
    read_side_map(env, account_id, POSITION_TYPE_DEPOSIT)
}

pub fn get_borrow_positions(env: &Env, account_id: u64) -> Map<Address, AccountPosition> {
    read_side_map(env, account_id, POSITION_TYPE_BORROW)
}

/// Single flush for the supply side. Removes the side key when the map is
/// empty; otherwise writes + bumps the side TTL. Use this when a batch
/// loaded the side map once, mutated in memory, and wants one final write.
pub fn set_supply_positions(
    env: &Env,
    account_id: u64,
    map: &Map<Address, AccountPosition>,
) {
    write_side_map(env, account_id, POSITION_TYPE_DEPOSIT, map);
}

/// Symmetric flush for the borrow side.
pub fn set_borrow_positions(
    env: &Env,
    account_id: u64,
    map: &Map<Address, AccountPosition>,
) {
    write_side_map(env, account_id, POSITION_TYPE_BORROW, map);
}

// ---------------------------------------------------------------------------
// Account-level lifecycle
// ---------------------------------------------------------------------------

pub fn get_account(env: &Env, account_id: u64) -> Account {
    try_get_account(env, account_id)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AccountNotFound))
}

pub fn try_get_account(env: &Env, account_id: u64) -> Option<Account> {
    try_get_account_meta(env, account_id).map(|meta| account_from_meta(env, account_id, &meta))
}

/// Compose-and-flush helper. Writes meta, supply map, and borrow map to
/// match `account` exactly. Used by entry points that mutate both sides
/// and want one final flush.
pub fn set_account(env: &Env, account_id: u64, account: &Account) {
    let meta = meta_from_account(account);
    set_account_meta(env, account_id, &meta);
    write_side_map(env, account_id, POSITION_TYPE_DEPOSIT, &account.supply_positions);
    write_side_map(env, account_id, POSITION_TYPE_BORROW, &account.borrow_positions);
}

/// Removes meta + both side maps. Idempotent.
pub fn remove_account_entry(env: &Env, account_id: u64) {
    let persistent = env.storage().persistent();
    persistent.remove(&account_meta_key(account_id));
    persistent.remove(&side_key(account_id, POSITION_TYPE_DEPOSIT));
    persistent.remove(&side_key(account_id, POSITION_TYPE_BORROW));
}

/// Keeps meta + both side maps alive without mutating them.
pub fn bump_account(env: &Env, account_id: u64) {
    let persistent = env.storage().persistent();
    let meta_key = account_meta_key(account_id);
    if persistent.has(&meta_key) {
        bump_user(env, &meta_key);
    }
    let supply_key = side_key(account_id, POSITION_TYPE_DEPOSIT);
    if persistent.has(&supply_key) {
        bump_user(env, &supply_key);
    }
    let borrow_key = side_key(account_id, POSITION_TYPE_BORROW);
    if persistent.has(&borrow_key) {
        bump_user(env, &borrow_key);
    }
}
