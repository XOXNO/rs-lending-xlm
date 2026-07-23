//! Core position money path: supply, borrow, withdraw, repay, liquidation.
//!
//! Auth and risk gates live on public entrypoints; pool settles shares/cash.
//! Stages: process_* → build_* → settle_* → finish_*_leg → finalize_position_flow.
//! See `architecture/INVARIANTS.md` §3.

use common::errors::{CollateralError, SpokeError};
use common::types::{
    Account, AccountPosition, AccountPositionType, DebtPosition, HubAssetKey, PoolAction,
    ScaledPositionRaw,
};
use soroban_sdk::{assert_with_error, panic_with_error, Env, Vec};

use crate::account;
use crate::context::Cache;
use crate::risk::validation;
use crate::spoke;
use crate::storage;

pub(crate) mod borrow;
pub(crate) mod liquidation;
pub(crate) mod repay;
pub(crate) mod supply;
pub(crate) mod withdraw;

/// Hub asset plus amount (one payment leg).
pub(crate) type HubPayment = (HubAssetKey, i128);

/// Deduped payment rows after aggregation.
pub(crate) type AggregatedPayments = Vec<HubPayment>;

/// Which account position maps to write on finalize.
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

pub(crate) fn persist_account_positions(
    env: &Env,
    account_id: u64,
    account: &Account,
    sides: PositionSides,
    remove_if_empty: bool,
) {
    if sides.supply {
        storage::set_supply_positions(env, account_id, &account.supply_positions);
    }
    if sides.debt {
        storage::set_debt_positions(env, account_id, &account.borrow_positions);
    }
    if remove_if_empty {
        account::cleanup_account_if_empty(env, account, account_id);
    }
}

/// Standard tail for user position flows: spoke usage, positions, then events.
///
/// `remove_if_empty` is true only on full-exit withdraw; supply/borrow/repay
/// leave the account in place even if one side is empty.
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

/// Shared pre-pool entry gates for deposit and borrow batches: hub active,
/// listing, pause/freeze flags, and side-specific supply/borrow capability.
pub(crate) fn validate_position_entry_gates(
    env: &Env,
    account: &Account,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
    position_type: AccountPositionType,
) {
    validation::validate_bulk_position_limits(env, account, position_type, aggregated);

    for (hub_asset, _) in aggregated {
        validation::require_hub_active(env, hub_asset.hub_id);
        // Unlisted assets revert `AssetNotInSpoke`.
        let asset_config =
            spoke::require_listed_active_config(env, cache, account.spoke_id, &hub_asset);
        // New entries: frozen blocks; paused blocks every verb.
        enforce_spoke_asset_flags(env, cache, account.spoke_id, &hub_asset, true);
        match position_type {
            AccountPositionType::Deposit => assert_with_error!(
                env,
                asset_config.can_supply(),
                CollateralError::NotCollateral
            ),
            AccountPositionType::Borrow => assert_with_error!(
                env,
                asset_config.can_borrow(),
                CollateralError::AssetNotBorrowable
            ),
        }
    }
}

/// Enforces per-spoke paused/frozen flags when the asset is still listed.
///
/// Always reverts if paused. When `block_when_frozen` is true (new deposit/borrow),
/// also reverts if frozen. Exit paths (withdraw/repay) pass false so freeze still
/// allows reducing positions. Missing listing is a no-op here (callers that need
/// a listing use `require_listed_active_config` first).
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

/// Supply position lookup for withdraw and related paths.
///
/// Panics with `CollateralPositionNotFound` (distinct from liquidation's
/// `expect_invariant` path so user errors stay stable).
pub(crate) fn get_supply_position_or_panic(
    env: &Env,
    account: &Account,
    hub_asset: &HubAssetKey,
) -> AccountPosition {
    (&account
        .supply_positions
        .get(hub_asset.clone())
        .unwrap_or_else(|| panic_with_error!(env, CollateralError::CollateralPositionNotFound)))
        .into()
}

/// Debt position lookup for repay and related paths.
///
/// Panics with `DebtPositionNotFound`.
pub(crate) fn get_debt_position_or_panic(
    env: &Env,
    account: &Account,
    hub_asset: &HubAssetKey,
) -> DebtPosition {
    (&account
        .borrow_positions
        .get(hub_asset.clone())
        .unwrap_or_else(|| panic_with_error!(env, CollateralError::DebtPositionNotFound)))
        .into()
}

#[cfg(test)]
#[path = "../../tests/positions/flags.rs"]
mod tests;
