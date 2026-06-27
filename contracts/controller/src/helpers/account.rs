//! In-memory account lifecycle helpers.
//!
//! Post-pool solvency gates (LTV, health factor, min borrow collateral) live in
//! `validation::require_post_pool_risk_gates`.

use common::errors::{EModeError, GenericError};
use common::math::fp::Ray;
use controller_interface::types::{
    Account, AccountMeta, AccountPosition, DebtPosition, PositionMode,
};
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env, Map};

use crate::cache::Cache;
use crate::storage;

/// Creates account metadata and returns an empty in-memory account snapshot.
///
/// When `cache` is provided, e-mode deprecation is checked via the transaction
/// cache so a later `AggregatedConfigs::resolve` does not re-read storage.
pub fn create_account(
    env: &Env,
    owner: &Address,
    e_mode_category: u32,
    mode: PositionMode,
    cache: &mut Cache,
) -> (u64, Account) {
    cache.active_e_mode_category(env, e_mode_category);

    let account_id = storage::increment_account_nonce(env);
    let account = Account {
        owner: owner.clone(),
        e_mode_category_id: e_mode_category,
        mode,
        supply_positions: Map::new(env),
        borrow_positions: Map::new(env),
    };
    storage::set_account_meta(
        env,
        account_id,
        &AccountMeta {
            owner: owner.clone(),
            e_mode_category_id: e_mode_category,
            mode,
        },
    );

    (account_id, account)
}

/// Existing-account guard applied by `load_or_create_account`, named for the
/// entrypoint whose check shape it encodes.
pub enum AccountGuard {
    /// Third-party supply: no owner check; a non-zero e-mode arg must match the
    /// stored category.
    Supply,
    /// Blend migration: caller must own the account; a non-zero e-mode arg must
    /// match the stored category.
    Migrate,
    /// Multiply strategy: caller must own the account; the stored position mode
    /// must equal `mode`.
    Multiply,
}

/// Loads an existing account or creates a new one when `account_id == 0`.
///
/// New accounts are created with `mode`; existing accounts are validated
/// against `guard`. The `mode` argument is only compared for the `Multiply` guard.
pub fn load_or_create_account(
    env: &Env,
    caller: &Address,
    account_id: u64,
    e_mode_category: u32,
    mode: PositionMode,
    guard: AccountGuard,
    cache: &mut Cache,
) -> (u64, Account) {
    if account_id == 0 {
        return create_account(env, caller, e_mode_category, mode, cache);
    }
    let account = storage::get_account(env, account_id);
    match guard {
        AccountGuard::Supply => require_emode_match(env, &account, e_mode_category),
        AccountGuard::Migrate => {
            crate::validation::require_account_owner_match(env, &account, caller);
            require_emode_match(env, &account, e_mode_category);
        }
        AccountGuard::Multiply => {
            crate::validation::require_account_owner_match(env, &account, caller);
            assert_with_error!(env, account.mode == mode, GenericError::AccountModeMismatch);
        }
    }
    (account_id, account)
}

/// Rejects a non-zero e-mode arg that conflicts with the stored category.
/// Zero is the unspecified sentinel; the stored category always governs.
fn require_emode_match(env: &Env, account: &Account, e_mode_category: u32) {
    if e_mode_category != 0 && e_mode_category != account.e_mode_category_id {
        panic_with_error!(env, EModeError::EModeMismatch);
    }
}

pub fn remove_account(env: &Env, account_id: u64) {
    storage::remove_account_entry(env, account_id);
}

pub fn cleanup_account_if_empty(env: &Env, account: &Account, account_id: u64) {
    if account.is_empty() {
        remove_account(env, account_id);
    }
}

/// Upserts collateral position or removes it when the scaled supply share is zero.
pub fn update_or_remove_supply_position(
    account: &mut Account,
    asset: &Address,
    position: &AccountPosition,
) -> bool {
    if position.scaled_amount == Ray::ZERO {
        account.supply_positions.remove(asset.clone());
        true
    } else {
        account.supply_positions.set(asset.clone(), position.into());
        false
    }
}

/// Upserts debt position or removes it when the scaled debt share is zero.
pub fn update_or_remove_debt_position(
    account: &mut Account,
    asset: &Address,
    position: &DebtPosition,
) -> bool {
    if position.scaled_amount == Ray::ZERO {
        account.borrow_positions.remove(asset.clone());
        true
    } else {
        account.borrow_positions.set(asset.clone(), position.into());
        false
    }
}
