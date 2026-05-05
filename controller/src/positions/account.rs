use common::types::{Account, PositionMode};
use soroban_sdk::{Address, Env, Map};

use super::emode;
use crate::storage;

/// Creates a new account, increments the nonce, persists it, and returns
/// the in-memory snapshot alongside the new id. Returning the snapshot
/// lets callers skip a redundant re-read for the entry that was just
/// written.
pub fn create_account(
    env: &Env,
    owner: &Address,
    e_mode_category: u32,
    mode: PositionMode,
    is_isolated: bool,
    isolated_asset: Option<Address>,
) -> (u64, Account) {
    emode::validate_e_mode_isolation_exclusion(env, e_mode_category, is_isolated);
    emode::active_e_mode_category(env, e_mode_category);

    let account_id = storage::increment_account_nonce(env);
    // The account nonce lives in instance storage; bump instance TTL on
    // every account creation so a long quiet period between governance
    // keepalives cannot let the nonce entry archive (which would reset
    // the next id back to 1 and collide with existing accounts).
    storage::bump_instance(env);
    let account = Account {
        owner: owner.clone(),
        is_isolated,
        e_mode_category_id: e_mode_category,
        mode,
        isolated_asset,
        supply_positions: Map::new(env),
        borrow_positions: Map::new(env),
    };
    storage::set_account(env, account_id, &account);

    (account_id, account)
}

/// Removes all persistent storage entries for `account_id` (meta entry and all positions).
pub fn remove_account(env: &Env, account_id: u64) {
    storage::remove_account_entry(env, account_id);
}

/// Removes the account from storage when both supply and borrow position maps are empty.
pub fn cleanup_account_if_empty(env: &Env, account: &Account, account_id: u64) {
    if account.supply_positions.is_empty() && account.borrow_positions.is_empty() {
        remove_account(env, account_id);
    }
}
