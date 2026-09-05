use common::errors::{GenericError, SpokeError};
use common::math::fp::Ray;
use common::types::{
    Account, AccountMeta, AccountPosition, DebtPosition, HubAssetKey, PositionMode,
};
use soroban_sdk::{
    assert_with_error, panic_with_error, Address, Env, IntoVal, Map, TryFromVal, Val,
};

use crate::context::Context;
use crate::events::AccountDelegateEvent;
use crate::external::position_nft::{nft_burn_call, nft_mint_call, nft_renew_call};
use crate::storage;

/// How strictly account creation treats a deprecated spoke.
#[derive(Clone, Copy)]
pub(crate) enum SpokeAdmission {
    /// New exposure: the spoke must be active.
    ActiveOnly,
    /// Seizure receivers may enter deprecated spokes to unwind existing exposure.
    AllowDeprecated,
}

/// Mints an account NFT and stores empty-account metadata in an active spoke.
pub(crate) fn create_account(
    env: &Env,
    owner: &Address,
    spoke_id: u32,
    mode: PositionMode,
    cache: &mut Context,
) -> (u64, Account) {
    create_account_with(
        env,
        owner,
        spoke_id,
        mode,
        cache,
        SpokeAdmission::ActiveOnly,
    )
}

/// Mints an account NFT and stores metadata. Requires an existing, nonzero spoke;
/// deprecated spokes are allowed only by `admission`.
pub(crate) fn create_account_with(
    env: &Env,
    owner: &Address,
    spoke_id: u32,
    mode: PositionMode,
    cache: &mut Context,
    admission: SpokeAdmission,
) -> (u64, Account) {
    assert_with_error!(env, spoke_id >= 1, SpokeError::SpokeNotFound);
    match admission {
        SpokeAdmission::ActiveOnly => {
            cache.active_spoke(spoke_id);
        }
        SpokeAdmission::AllowDeprecated => {
            let _ = cache.spoke_config(spoke_id);
        }
    }

    let nft = storage::get_position_nft(env);
    let account_id = nft_mint_call(env, &nft, owner);
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

/// Additional checks when reusing an existing account.
pub(crate) enum AccountGuard {
    Supply,
    Migrate,
    Multiply,
}

/// Creates an account for `caller` when `account_id` is zero; otherwise loads it.
/// Existing accounts require a matching spoke. `Migrate` also requires an owner
/// or active delegate; `Multiply` additionally requires a matching mode.
pub(crate) fn load_or_create_account(
    env: &Env,
    caller: &Address,
    account_id: u64,
    spoke_id: u32,
    mode: PositionMode,
    guard: AccountGuard,
    cache: &mut Context,
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

/// Accepts the owner or a registered, active manager delegated by that owner.
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

/// Requires the owner or a registered, active manager delegated by that owner.
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

/// Returns metadata after verifying that `caller` currently owns the account NFT.
pub(crate) fn require_account_owner(env: &Env, account_id: u64, caller: &Address) -> AccountMeta {
    let meta = storage::get_account_meta(env, account_id);
    let owner = storage::account_owner(env, account_id);
    assert_with_error!(env, owner == *caller, GenericError::AccountNotInMarket);
    meta
}

/// Requires the account to belong to `spoke_id`.
fn require_spoke_match(env: &Env, account: &Account, spoke_id: u32) {
    if spoke_id != account.spoke_id {
        panic_with_error!(env, SpokeError::SpokeMismatch);
    }
}

/// Deletes all account entries and burns its NFT atomically. Account deletion
/// must use this path to preserve the NFT/account existence invariant.
pub(crate) fn remove_account_and_burn_nft(env: &Env, account_id: u64) {
    storage::remove_account_entry(env, account_id);
    let nft = storage::get_position_nft(env);
    nft_burn_call(env, &nft, account_id);
}

/// Deletes the account and burns its NFT when both position maps are empty.
pub(crate) fn cleanup_account_if_empty(env: &Env, account: &Account, account_id: u64) {
    if account.is_empty() {
        remove_account_and_burn_nft(env, account_id);
    }
}

fn upsert_or_remove_position<V>(
    map: &mut Map<HubAssetKey, V>,
    hub_asset: &HubAssetKey,
    value: Option<V>,
) where
    V: IntoVal<Env, Val> + TryFromVal<Env, Val>,
{
    match value {
        None => {
            map.remove(hub_asset.clone());
        }
        Some(value) => {
            map.set(hub_asset.clone(), value);
        }
    }
}

/// Updates the in-memory supply position, removing it when scaled supply is zero.
pub(crate) fn update_or_remove_supply_position(
    account: &mut Account,
    hub_asset: &HubAssetKey,
    position: &AccountPosition,
) {
    upsert_or_remove_position(
        &mut account.supply_positions,
        hub_asset,
        (position.scaled_amount != Ray::ZERO).then(|| position.into()),
    );
}

/// Updates the in-memory debt position, removing it when scaled debt is zero.
pub(crate) fn update_or_remove_debt_position(
    account: &mut Account,
    hub_asset: &HubAssetKey,
    position: &DebtPosition,
) {
    upsert_or_remove_position(
        &mut account.borrow_positions,
        hub_asset,
        (position.scaled_amount != Ray::ZERO).then(|| position.into()),
    );
}

/// Requires the NFT owner to authorize renewal of the instance and account entries.
/// Renews the NFT owner entry to the same user TTL window (INV-STOR-02).
pub(crate) fn renew_account(env: &Env, caller: Address, account_id: u64) {
    storage::renew_controller_instance(env);

    caller.require_auth();
    require_account_owner(env, account_id, &caller);

    storage::renew_user_account(env, account_id);
    let nft = storage::get_position_nft(env);
    nft_renew_call(env, &nft, account_id);
}

/// Requires the NFT owner to grant an active manager access; renews instance TTL.
pub(crate) fn add_delegate(env: &Env, caller: Address, account_id: u64, delegate: Address) {
    storage::renew_controller_instance(env);
    set_account_delegate(env, &caller, account_id, &delegate, true);
}

/// Requires the NFT owner to revoke manager access; renews instance TTL.
pub(crate) fn remove_delegate(env: &Env, caller: Address, account_id: u64, delegate: Address) {
    storage::renew_controller_instance(env);
    set_account_delegate(env, &caller, account_id, &delegate, false);
}

/// Authenticates the NFT owner and updates delegates; grants require an active
/// manager. Emits an event only when the current owner's delegate list changes.
fn set_account_delegate(
    env: &Env,
    caller: &Address,
    account_id: u64,
    delegate: &Address,
    add: bool,
) {
    caller.require_auth();
    require_account_owner(env, account_id, caller);
    if add {
        // Reject dormant grants that could gain authority on later manager activation.
        assert_with_error!(
            env,
            storage::get_position_manager(env, delegate).is_some_and(|c| c.is_active),
            GenericError::NotAuthorized
        );
    }

    let changed = if add {
        storage::add_delegate(env, account_id, caller, delegate)
    } else {
        storage::remove_delegate(env, account_id, caller, delegate)
    };

    if changed {
        AccountDelegateEvent {
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
