//! Account creation, ownership/delegate authorization, position bookkeeping, and
//! delegate-management entrypoints for the controller.

use common::errors::{GenericError, SpokeError};
use common::math::fp::Ray;
use common::types::{
    Account, AccountMeta, AccountPosition, DebtPosition, HubAssetKey, PositionMode,
};
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env, Map};

use crate::context::Cache;
use crate::storage;

/// Allocates a new account nonce, requires the target spoke to be active, and
/// persists the account's meta record. Returns the new account id together with
/// an in-memory `Account` with empty supply and borrow positions. Panics if
/// `spoke_id` is 0 or if the spoke is not active.
pub(crate) fn create_account(
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

/// Selects which checks [`load_or_create_account`] applies to an existing account
/// before returning it.
pub(crate) enum AccountGuard {
    /// Requires only that the account's spoke matches the requested spoke.
    Supply,
    /// Requires the caller to be the owner, or an active protocol position manager
    /// listed among the account's delegates, and requires the account's spoke to
    /// match the requested spoke.
    Migrate,
    /// Requires the caller to be the owner, or an active protocol position manager
    /// listed among the account's delegates, requires the account's spoke to match
    /// the requested spoke, and requires the account's mode to match the requested
    /// mode.
    Multiply,
}

/// Returns the account for `account_id`, creating a new one via [`create_account`]
/// when `account_id` is 0. For an existing account, applies the checks selected by
/// `guard` and panics via [`require_spoke_match`], [`require_owner_or_delegate`],
/// or with [`GenericError::AccountModeMismatch`] if they fail.
pub(crate) fn load_or_create_account(
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
            require_spoke_match(env, &account, spoke_id);
            assert_with_error!(env, account.mode == mode, GenericError::AccountModeMismatch);
        }
    }
    (account_id, account)
}

/// Returns whether `caller` equals `owner`, or is an active position manager
/// listed among `account_id`'s delegates.
pub(crate) fn is_owner_or_delegate(
    env: &Env,
    account_id: u64,
    caller: &Address,
    owner: &Address,
) -> bool {
    if caller == owner {
        return true;
    }
    let active_manager =
        storage::get_position_manager(env, caller).is_some_and(|config| config.is_active);
    active_manager && storage::get_delegates(env, account_id).contains(caller)
}

/// Panics with [`GenericError::NotAuthorized`] unless `caller` is the owner, or
/// is both an active protocol position manager and listed among `account_id`'s
/// delegates.
pub(crate) fn require_owner_or_delegate(
    env: &Env,
    account_id: u64,
    caller: &Address,
    owner: &Address,
) {
    if is_owner_or_delegate(env, account_id, caller, owner) {
        return;
    }
    panic_with_error!(env, GenericError::NotAuthorized);
}

/// Loads the account meta for `account_id` and requires `caller` matches the
/// owner. Missing accounts and non-owner callers both panic with
/// [`GenericError::AccountNotInMarket`] so owner-only entrypoints do not leak
/// whether an account exists or who owns it.
pub(crate) fn require_account_owner(env: &Env, account_id: u64, caller: &Address) -> AccountMeta {
    let meta = storage::get_account_meta(env, account_id);
    assert_with_error!(env, meta.owner == *caller, GenericError::AccountNotInMarket);
    meta
}

/// Panics with [`SpokeError::SpokeMismatch`] if `spoke_id` differs from
/// `account.spoke_id`.
fn require_spoke_match(env: &Env, account: &Account, spoke_id: u32) {
    if spoke_id != account.spoke_id {
        panic_with_error!(env, SpokeError::SpokeMismatch);
    }
}

/// Removes the storage entry for `account_id` if `account` has no supply and no
/// borrow positions.
pub(crate) fn cleanup_account_if_empty(env: &Env, account: &Account, account_id: u64) {
    if account.is_empty() {
        storage::remove_account_entry(env, account_id);
    }
}

/// Sets `account`'s supply position for `hub_asset` to `position`, or removes the
/// entry if `position.scaled_amount` is zero.
pub(crate) fn update_or_remove_supply_position(
    account: &mut Account,
    hub_asset: &HubAssetKey,
    position: &AccountPosition,
) {
    if position.scaled_amount == Ray::ZERO {
        account.supply_positions.remove(hub_asset.clone());
    } else {
        account
            .supply_positions
            .set(hub_asset.clone(), position.into());
    }
}

/// Sets `account`'s borrow position for `hub_asset` to `position`, or removes the
/// entry if `position.scaled_amount` is zero.
pub(crate) fn update_or_remove_debt_position(
    account: &mut Account,
    hub_asset: &HubAssetKey,
    position: &DebtPosition,
) {
    if position.scaled_amount == Ray::ZERO {
        account.borrow_positions.remove(hub_asset.clone());
    } else {
        account
            .borrow_positions
            .set(hub_asset.clone(), position.into());
    }
}

/// Renews the controller instance's TTL, requires `caller`'s authorization and
/// ownership of `account_id`, and renews the account's storage TTL.
pub(crate) fn renew_account(env: &Env, caller: Address, account_id: u64) {
    storage::renew_controller_instance(env);

    caller.require_auth();
    let _meta = require_account_owner(env, account_id, &caller);

    storage::renew_user_account(env, account_id);
}

/// Renews the controller instance's TTL and grants `delegate` as a delegate of
/// `account_id`, requiring `caller`'s authorization and ownership.
pub(crate) fn add_delegate(env: &Env, caller: Address, account_id: u64, delegate: Address) {
    storage::renew_controller_instance(env);
    set_account_delegate(env, &caller, account_id, &delegate, true);
}

/// Renews the controller instance's TTL and revokes `delegate` as a delegate of
/// `account_id`, requiring `caller`'s authorization and ownership.
pub(crate) fn remove_delegate(env: &Env, caller: Address, account_id: u64, delegate: Address) {
    storage::renew_controller_instance(env);
    set_account_delegate(env, &caller, account_id, &delegate, false);
}

/// Requires `caller`'s authorization and ownership of `account_id`, then adds or
/// removes `delegate` from the account's delegate list depending on `add`.
/// Publishes an [`crate::events::AccountDelegateEvent`] if the delegate list
/// actually changes.
fn set_account_delegate(
    env: &Env,
    caller: &Address,
    account_id: u64,
    delegate: &Address,
    add: bool,
) {
    caller.require_auth();
    let meta = require_account_owner(env, account_id, caller);
    if add {
        // Grant and activation must be contemporaneous: a dormant grant to an
        // address governance has not yet approved would arm on activation.
        assert_with_error!(
            env,
            storage::get_position_manager(env, delegate).is_some_and(|c| c.is_active),
            GenericError::NotAuthorized
        );
    }

    let changed = if add {
        storage::add_delegate(env, account_id, delegate)
    } else {
        storage::remove_delegate(env, account_id, delegate)
    };

    if changed {
        crate::events::AccountDelegateEvent {
            account_id,
            owner: meta.owner,
            delegate: delegate.clone(),
            granted: add,
        }
        .publish(env);
    }
}

#[cfg(test)]
#[path = "../../tests/helpers/account.rs"]
mod tests;
