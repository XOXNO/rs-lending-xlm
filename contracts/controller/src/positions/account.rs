use common::types::{Account, AccountMeta, PositionMode};
use soroban_sdk::{Address, Env, Map};

use super::emode;
use crate::storage;

// Creates and persists new account.
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
    let account = Account {
        owner: owner.clone(),
        is_isolated,
        e_mode_category_id: e_mode_category,
        mode,
        isolated_asset,
        supply_positions: Map::new(env),
        borrow_positions: Map::new(env),
    };
    storage::set_account_meta(
        env,
        account_id,
        &AccountMeta {
            owner: owner.clone(),
            is_isolated,
            e_mode_category_id: e_mode_category,
            mode,
            isolated_asset: account.isolated_asset.clone(),
        },
    );

    (account_id, account)
}

// Deletes account from storage.
pub fn remove_account(env: &Env, account_id: u64) {
    storage::remove_account_entry(env, account_id);
}

// Deletes account if empty.
pub fn cleanup_account_if_empty(env: &Env, account: &Account, account_id: u64) {
    if account.supply_positions.is_empty() && account.borrow_positions.is_empty() {
        remove_account(env, account_id);
    }
}
