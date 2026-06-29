//! Core position lifecycle operations.
//!
//! Each submodule owns one public position flow and its `process_*` pipeline.
//! Shared stages are auth, cache setup, account resolution, validation, pool
//! calls, post-checks, then `finalize_position_flow` (or `persist_account_positions`
//! + `emit_account_updates` when a hook is needed, e.g. liquidation bad-debt).

use common::errors::{CollateralError, SpokeError};
use controller_interface::types::{
    Account, AccountPosition, DebtPosition, HubAssetKey, PoolAction, ScaledPositionRaw,
};
use soroban_sdk::{assert_with_error, panic_with_error, Env, Vec};

use crate::cache::Cache;
use crate::helpers;
use crate::storage;

pub mod borrow;
pub mod liquidation;
pub mod liquidation_math;
pub mod repay;
pub mod supply;
pub mod withdraw;

/// One re-keyed payment row: hub asset coordinate plus amount.
pub(crate) type HubPayment = (HubAssetKey, i128);

/// Deduped payment rows: one entry per hub asset with the summed amount for the call.
pub(crate) type AggregatedPayments = Vec<HubPayment>;

/// Which position maps to persist at the end of a flow.
#[derive(Copy, Clone)]
pub(crate) struct PositionSides {
    pub supply: bool,
    pub debt: bool,
}

impl PositionSides {
    pub const SUPPLY: Self = Self {
        supply: true,
        debt: false,
    };
    pub const DEBT: Self = Self {
        supply: false,
        debt: true,
    };
    pub const BOTH: Self = Self {
        supply: true,
        debt: true,
    };
}

/// Writes supply and/or debt maps, or removes the account when `remove_if_empty`
/// and the snapshot has no positions.
pub(crate) fn persist_account_positions(
    env: &Env,
    account_id: u64,
    account: &Account,
    sides: PositionSides,
    remove_if_empty: bool,
) {
    if remove_if_empty && account.is_empty() {
        helpers::remove_account(env, account_id);
        return;
    }
    if sides.supply {
        storage::set_supply_positions(env, account_id, &account.supply_positions);
    }
    if sides.debt {
        storage::set_debt_positions(env, account_id, &account.borrow_positions);
    }
}

/// Standard tail for user position flows: persist then emit.
pub(crate) fn finalize_position_flow(
    env: &Env,
    account_id: u64,
    account: &Account,
    cache: &mut Cache,
    sides: PositionSides,
    remove_if_empty: bool,
) {
    cache.persist_spoke_usage();
    persist_account_positions(env, account_id, account, sides, remove_if_empty);
    cache.emit_position_batch(account_id, account);
}

/// Enforces the active spoke's per-asset trading flags. `paused` blocks every
/// verb; `frozen` blocks only new supply/borrow (`block_when_frozen`). No-op
/// when the asset has no entry on the spoke (e.g. an asset de-listed from the
/// spoke while a position survives), so exits stay unblocked.
pub(crate) fn enforce_spoke_asset_flags(
    env: &Env,
    cache: &mut Cache,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
    block_when_frozen: bool,
) {
    if let Some(sa) = cache.cached_spoke_asset(spoke_id, hub_asset) {
        assert_with_error!(env, !sa.paused, SpokeError::SpokeAssetPaused);
        if block_when_frozen {
            assert_with_error!(env, !sa.frozen, SpokeError::SpokeAssetFrozen);
        }
    }
}

/// Pure construction helper for the repeated `PoolAction` literal used in each
/// bulk pool entry path. Preserves exact semantics and Into behavior.
pub(crate) fn make_pool_action(
    position: impl Into<ScaledPositionRaw>,
    amount: i128,
    hub_asset: HubAssetKey,
) -> PoolAction {
    PoolAction {
        position: position.into(),
        amount,
        hub_asset,
    }
}

/// Exact lookup for user-facing repay/withdraw paths. Kept separate from
/// `expect_invariant` liquidation apply paths to preserve missing-position
/// error codes.
pub(crate) fn get_supply_position_or_panic(
    env: &Env,
    account: &Account,
    hub_asset: &HubAssetKey,
) -> AccountPosition {
    (&account
        .supply_positions
        .get(hub_asset.clone())
        .unwrap_or_else(|| panic_with_error!(env, CollateralError::PositionNotFound)))
        .into()
}

pub(crate) fn get_debt_position_or_panic(
    env: &Env,
    account: &Account,
    hub_asset: &HubAssetKey,
) -> DebtPosition {
    (&account
        .borrow_positions
        .get(hub_asset.clone())
        .unwrap_or_else(|| panic_with_error!(env, CollateralError::PositionNotFound)))
        .into()
}

#[cfg(test)]
#[path = "../../tests/positions/flags.rs"]
mod tests;
