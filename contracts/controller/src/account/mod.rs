//! Account lifecycle helpers.

pub(crate) mod delegation;

use common::errors::{GenericError, SpokeError};
use common::math::fp::Ray;
use common::types::{
    Account, AccountMeta, AccountPosition, DebtPosition, HubAssetKey, PositionMode,
};
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env, Map};

use crate::context::Cache;
use crate::storage;

/// Creates account metadata and an empty in-memory account for an active spoke.
pub fn create_account(
    env: &Env,
    owner: &Address,
    spoke_id: u32,
    mode: PositionMode,
    cache: &mut Cache,
) -> (u64, Account) {
    assert_with_error!(env, spoke_id >= 1, SpokeError::SpokeNotFound);
    cache.active_spoke(spoke_id);

    let account_id = storage::increment_account_nonce(env);
    let account = Account {
        owner: owner.clone(),
        spoke_id,
        mode,
        supply_positions: Map::new(env),
        borrow_positions: Map::new(env),
    };
    storage::set_account_meta(
        env,
        account_id,
        &AccountMeta {
            owner: owner.clone(),
            spoke_id,
            mode,
        },
    );

    (account_id, account)
}

/// Existing-account guard for `load_or_create_account`.
pub enum AccountGuard {
    /// Third-party supply; spoke arg must match stored spoke.
    Supply,
    /// Blend migration; caller must be owner or an active delegate, and spoke must match.
    Migrate,
    /// Multiply strategy; caller must be owner or an active delegate, and mode must match.
    Multiply,
}

/// Loads existing account or creates a new one when `account_id == 0`.
pub fn load_or_create_account(
    env: &Env,
    caller: &Address,
    account_id: u64,
    spoke_id: u32,
    mode: PositionMode,
    guard: AccountGuard,
    cache: &mut Cache,
) -> (u64, Account) {
    if account_id == 0 {
        return create_account(env, caller, spoke_id, mode, cache);
    }
    let account = storage::get_account(env, account_id);
    match guard {
        AccountGuard::Supply => require_spoke_match(env, &account, spoke_id),
        AccountGuard::Migrate => {
            require_owner_or_delegate(env, account_id, caller, &account.owner);
            require_spoke_match(env, &account, spoke_id);
        }
        AccountGuard::Multiply => {
            require_owner_or_delegate(env, account_id, caller, &account.owner);
            assert_with_error!(env, account.mode == mode, GenericError::AccountModeMismatch);
        }
    }
    (account_id, account)
}

/// Requires caller to be owner or an active registered delegate.
pub fn require_owner_or_delegate(env: &Env, account_id: u64, caller: &Address, owner: &Address) {
    if caller == owner {
        return;
    }
    let active_manager =
        storage::get_position_manager(env, caller).is_some_and(|config| config.is_active);
    if active_manager && storage::get_delegates(env, account_id).contains(caller.clone()) {
        return;
    }
    panic_with_error!(env, GenericError::NotAuthorized);
}

/// Rejects a spoke arg that does not match the account's stored spoke.
fn require_spoke_match(env: &Env, account: &Account, spoke_id: u32) {
    if spoke_id != account.spoke_id {
        panic_with_error!(env, SpokeError::SpokeMismatch);
    }
}

/// Removes the account entry from storage when it holds no positions.
pub fn cleanup_account_if_empty(env: &Env, account: &Account, account_id: u64) {
    if account.is_empty() {
        storage::remove_account_entry(env, account_id);
    }
}

/// Upserts collateral position or removes it when the scaled supply share is zero.
pub fn update_or_remove_supply_position(
    account: &mut Account,
    hub_asset: &HubAssetKey,
    position: &AccountPosition,
) -> bool {
    if position.scaled_amount == Ray::ZERO {
        account.supply_positions.remove(hub_asset.clone());
        true
    } else {
        account
            .supply_positions
            .set(hub_asset.clone(), position.into());
        false
    }
}

/// Upserts debt position or removes it when the scaled debt share is zero.
pub fn update_or_remove_debt_position(
    account: &mut Account,
    hub_asset: &HubAssetKey,
    position: &DebtPosition,
) -> bool {
    if position.scaled_amount == Ray::ZERO {
        account.borrow_positions.remove(hub_asset.clone());
        true
    } else {
        account
            .borrow_positions
            .set(hub_asset.clone(), position.into());
        false
    }
}

#[cfg(test)]
#[path = "../../tests/helpers/account.rs"]
mod tests;
