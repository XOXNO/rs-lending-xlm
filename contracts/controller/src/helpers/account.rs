//! In-memory account lifecycle helpers.
//!
//! Post-pool solvency gates (LTV, health factor, min borrow collateral) live in
//! `validation::require_post_pool_risk_gates`.

use common::math::fp::Ray;
use controller_interface::types::{
    Account, AccountMeta, AccountPosition, DebtPosition, PositionMode,
};
use soroban_sdk::{Address, Env, Map};

use crate::cache::Cache;
use crate::emode;
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
    cache: Option<&mut Cache>,
) -> (u64, Account) {
    if let Some(cache) = cache {
        cache.active_e_mode_category(env, e_mode_category);
    } else {
        emode::active_e_mode_category(env, e_mode_category);
    }

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
