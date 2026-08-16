use common::errors::{GenericError, SpokeError};
use common::math::fp::Ray;
use common::types::{
    Account, AccountMeta, AccountPosition, DebtPosition, HubAssetKey, PositionMode,
};
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env, Map};

use crate::context::Cache;
use crate::storage;

/// Creates a new account owned by `owner` in `spoke_id`, assigning it a fresh account id
/// and persisting its metadata. Panics if `spoke_id` is 0, unknown, or deprecated.
pub(crate) fn create_account(
    env: &Env,
    owner: &Address,
    spoke_id: u32,
    mode: PositionMode,
    cache: &mut Cache,
) -> (u64, Account) {
    assert_with_error!(env, spoke_id >= 1, SpokeError::SpokeNotFound);
    cache.active_spoke(spoke_id);

    let nft = storage::get_position_nft(env);
    let account_id = crate::external::position_nft::nft_mint_call(env, &nft, owner);
    let account = Account {
        owner: owner.clone(),
        spoke_id,
        mode,
        supply_positions: Map::new(env),
        borrow_positions: Map::new(env),
    };
    storage::set_account_meta(env, account_id, &AccountMeta { spoke_id, mode });

    (account_id, account)
}

pub(crate) enum AccountGuard {
    Supply,
    Migrate,
    Multiply,
}

/// Creates a new account when `account_id` is 0, otherwise loads the existing account and
/// enforces `guard`: `Supply` checks the spoke matches; `Migrate` additionally requires
/// owner-or-delegate authorization; `Multiply` further requires the account's mode to match `mode`.
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

/// Returns whether `caller` is `owner`, or is an active position manager registered as a
/// delegate on `account_id`.
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
    active_manager && storage::get_delegates(env, account_id, owner).contains(caller)
}

/// Panics unless `caller` is `owner` or an active delegate on `account_id`.
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

/// Returns the account's stored metadata, panicking unless `caller` currently owns the
/// account's position NFT.
pub(crate) fn require_account_owner(env: &Env, account_id: u64, caller: &Address) -> AccountMeta {
    let meta = storage::get_account_meta(env, account_id);
    let owner = storage::account_owner(env, account_id);
    assert_with_error!(env, owner == *caller, GenericError::AccountNotInMarket);
    meta
}

/// Panics unless `account`'s spoke id equals `spoke_id`.
fn require_spoke_match(env: &Env, account: &Account, spoke_id: u32) {
    if spoke_id != account.spoke_id {
        panic_with_error!(env, SpokeError::SpokeMismatch);
    }
}

/// Removes the account's stored entry and burns its position NFT if it has no supply or
/// borrow positions left.
pub(crate) fn cleanup_account_if_empty(env: &Env, account: &Account, account_id: u64) {
    if account.is_empty() {
        storage::remove_account_entry(env, account_id);
        let nft = storage::get_position_nft(env);
        crate::external::position_nft::nft_burn_call(env, &nft, account_id);
    }
}

/// Sets `account`'s supply position for `hub_asset` to `position` in memory, removing the
/// entry instead if its scaled amount is zero.
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

/// Sets `account`'s debt position for `hub_asset` to `position` in memory, removing the
/// entry instead if its scaled amount is zero.
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

/// Extends the TTL of the controller instance, then, after requiring `caller`'s
/// authorization and ownership of `account_id`, extends the TTL of the account's stored entries.
pub(crate) fn renew_account(env: &Env, caller: Address, account_id: u64) {
    storage::renew_controller_instance(env);

    caller.require_auth();
    let _meta = require_account_owner(env, account_id, &caller);

    storage::renew_user_account(env, account_id);
}

/// Extends the controller instance's TTL and grants `delegate` authorization to act on
/// `account_id` on `caller`'s behalf.
pub(crate) fn add_delegate(env: &Env, caller: Address, account_id: u64, delegate: Address) {
    storage::renew_controller_instance(env);
    set_account_delegate(env, &caller, account_id, &delegate, true);
}

/// Extends the controller instance's TTL and revokes `delegate`'s authorization to act on
/// `account_id` on `caller`'s behalf.
pub(crate) fn remove_delegate(env: &Env, caller: Address, account_id: u64, delegate: Address) {
    storage::renew_controller_instance(env);
    set_account_delegate(env, &caller, account_id, &delegate, false);
}

/// Requires `caller`'s authorization and ownership of `account_id`, then adds or removes
/// `delegate` from its delegate list depending on `add`, requiring an active position
/// manager when adding. Publishes an `AccountDelegateEvent` if the delegate list changed.
fn set_account_delegate(
    env: &Env,
    caller: &Address,
    account_id: u64,
    delegate: &Address,
    add: bool,
) {
    caller.require_auth();
    let _meta = require_account_owner(env, account_id, caller);
    if add {
        // Grant and activation must be contemporaneous: a dormant grant to an
        // address governance has not yet approved would arm on activation.
        assert_with_error!(
            env,
            storage::get_position_manager(env, delegate).is_some_and(|c| c.is_active),
            GenericError::NotAuthorized
        );
    }

    // `require_account_owner` already established `caller` as the current owner.
    let changed = if add {
        storage::add_delegate(env, account_id, caller, delegate)
    } else {
        storage::remove_delegate(env, account_id, caller, delegate)
    };

    if changed {
        crate::events::AccountDelegateEvent {
            account_id,
            owner: caller.clone(),
            delegate: delegate.clone(),
            granted: add,
        }
        .publish(env);
    }
}

#[cfg(test)]
#[path = "../tests/helpers/account.rs"]
mod tests;
