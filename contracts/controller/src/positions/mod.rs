//! Shared position gates, pool-result merging, spoke usage, and persistence
//! for supply, debt, liquidation, and strategy flows.

pub(crate) mod debt;

use common::errors::{CollateralError, FlashLoanError, GenericError, SpokeError};
use common::math::fp::Ray;
use common::types::{
    Account, AccountPosition, AccountPositionType, AggregatedPayments, AssetConfig, DebtPosition,
    HubAssetKey, HubPayment, MarketIndexRaw, PoolAction, PoolPositionMutation, ScaledPositionRaw,
};
use soroban_sdk::{
    assert_with_error, panic_with_error, Address, Env, IntoVal, TryFromVal, Val, Vec,
};

use crate::account;
use crate::context::Context;
use crate::risk::{self, validation};
use crate::spoke_usage::UsageSide;
use crate::storage;

pub(crate) mod liquidation;
pub(crate) use debt::{
    apply_repay_batch, borrow_into_controller, merge_debt_leg, process_borrow, process_repay,
    repay_prefunded_position, RepaymentRequest,
};
pub(crate) use supply::{
    apply_withdraw_batch, execute_withdrawal, merge_withdraw_leg, process_supply, process_withdraw,
    WithdrawKind, WithdrawalRequest,
};
pub(crate) mod supply;

/// Rejects pool and controller recipients with `InvalidFlashloanReceiver`.
/// Pool self-transfers debit cash without moving tokens; controller receipts
/// would remain unclaimed by balance-delta accounting.
pub(crate) fn require_external_recipient(env: &Env, cache: &mut Context, recipient: &Address) {
    let pool = cache.cached_pool_address();
    assert_with_error!(
        env,
        *recipient != env.current_contract_address() && *recipient != pool,
        FlashLoanError::InvalidFlashloanReceiver
    );
}

pub(crate) struct LegOutcome {
    pub new_scaled: Ray,
    pub market_index: MarketIndexRaw,
    pub amount: i128,
}

impl From<&PoolPositionMutation> for LegOutcome {
    fn from(mutation: &PoolPositionMutation) -> Self {
        Self {
            new_scaled: Ray::from(mutation.position.scaled_amount),
            market_index: mutation.market_index.clone(),
            amount: mutation.actual_amount,
        }
    }
}

/// Pairs requests and pool results in order; rejects mismatched lengths.
pub(crate) fn for_each_leg<E, R>(
    env: &Env,
    entries: &Vec<E>,
    results: &Vec<R>,
    mut f: impl FnMut(E, R),
) where
    E: IntoVal<Env, Val> + TryFromVal<Env, Val> + Clone,
    R: IntoVal<Env, Val> + TryFromVal<Env, Val> + Clone,
{
    assert_with_error!(
        env,
        results.len() == entries.len(),
        GenericError::InternalError
    );
    for (entry, result) in entries.iter().zip(results.iter()) {
        f(entry, result);
    }
}

/// Restamps listed supply LTVs, then checks collateral coverage, health factor,
/// and minimum borrow collateral. Returns whether any LTV changed.
pub(crate) fn enforce_post_pool_solvency(
    env: &Env,
    cache: &mut Context,
    account: &mut Account,
) -> bool {
    let restamped = risk::restamp_listed_supply_ltv(cache, account);
    validation::require_post_pool_risk_gates(env, cache, account);
    restamped
}

#[derive(Clone, Copy)]
pub(crate) enum LegDirection {
    Entry { asset_decimals: u32 },
    Exit,
}

/// Applies the scaled position delta to spoke usage; entries enforce caps
/// using the leg's market index and asset decimals.
pub(crate) fn apply_leg_usage(
    env: &Env,
    cache: &mut Context,
    spoke_id: u32,
    side: UsageSide,
    hub_asset: &HubAssetKey,
    direction: LegDirection,
    old_scaled: Ray,
    outcome: &LegOutcome,
) {
    match direction {
        LegDirection::Entry { asset_decimals } => cache.apply_spoke_entry(
            spoke_id,
            side,
            hub_asset,
            outcome.new_scaled.checked_sub(env, old_scaled),
            &outcome.market_index,
            asset_decimals,
        ),
        LegDirection::Exit => cache.apply_spoke_exit(
            spoke_id,
            side,
            hub_asset,
            old_scaled.checked_sub(env, outcome.new_scaled),
        ),
    }
}

/// Listing halt policy for each leg.
///
/// Seizure uses `no_seize`: applying `paused` to pro-rata seizure would block
/// liquidation of every account holding the paused collateral (ADR-0008).
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum FreezePolicy {
    /// New exposure: rejects `paused` and `frozen`.
    BlockOnEntry,
    /// User-initiated exit: rejects `paused`, tolerates `frozen`.
    AllowOnExit,
    /// Liquidation seizure: rejects `no_seize` only, tolerates `paused` and `frozen`.
    SeizureLeg,
}

/// Which position maps a flow writes back to storage.
#[derive(Copy, Clone, PartialEq)]
pub(crate) enum PositionSides {
    Supply,
    Debt,
    Both,
}

/// Persists the selected position maps, renews account TTL, and optionally
/// removes an empty account.
pub(crate) fn persist_account_positions(
    env: &Env,
    account_id: u64,
    account: &Account,
    sides: PositionSides,
    remove_if_empty: bool,
) {
    if sides != PositionSides::Debt {
        storage::set_supply_positions(env, account_id, &account.supply_positions);
    }
    if sides != PositionSides::Supply {
        storage::set_debt_positions(env, account_id, &account.borrow_positions);
    }
    storage::renew_user_account(env, account_id);
    if remove_if_empty {
        account::cleanup_account_if_empty(env, account, account_id);
    }
}

/// Persists spoke usage and positions, then emits the position-update batch.
pub(crate) fn finalize_position_flow(
    env: &Env,
    account_id: u64,
    account: &Account,
    cache: &mut Context,
    sides: PositionSides,
    remove_if_empty: bool,
) {
    cache.persist_spoke_usage();
    persist_account_positions(env, account_id, account, sides, remove_if_empty);
    cache.emit_position_batch(account_id, account);
}

/// Requires an active hub, an active spoke listing, and neither pause nor
/// freeze. Returns the config for the supply or borrow permission check.
fn require_listed_unhalted_config(
    env: &Env,
    cache: &mut Context,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
) -> AssetConfig {
    cache.require_hub_active(hub_asset.hub_id);
    let asset_config = cache.require_listed_active_config(spoke_id, hub_asset);
    enforce_spoke_asset_flags(env, cache, spoke_id, hub_asset, FreezePolicy::BlockOnEntry);
    asset_config
}

/// Requires an active, unhalted listing that permits borrowing.
pub(crate) fn require_can_borrow(
    env: &Env,
    cache: &mut Context,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
) {
    let asset_config = require_listed_unhalted_config(env, cache, spoke_id, hub_asset);
    assert_with_error!(
        env,
        asset_config.is_borrowable,
        CollateralError::AssetNotBorrowable
    );
}

/// Requires an active, unhalted listing that permits collateral supply.
pub(crate) fn require_can_supply(
    env: &Env,
    cache: &mut Context,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
) {
    let asset_config = require_listed_unhalted_config(env, cache, spoke_id, hub_asset);
    assert_with_error!(
        env,
        asset_config.is_collateralizable,
        CollateralError::NotCollateral
    );
}

/// Checks position-count limits and entry permissions for all aggregated assets.
pub(crate) fn validate_position_entry_gates(
    env: &Env,
    account: &Account,
    aggregated: &AggregatedPayments,
    cache: &mut Context,
    position_type: AccountPositionType,
) {
    validation::validate_bulk_position_limits(env, account, position_type, aggregated);

    for (hub_asset, _) in aggregated {
        match position_type {
            AccountPositionType::Deposit => {
                require_can_supply(env, cache, account.spoke_id, &hub_asset);
            }
            AccountPositionType::Borrow => {
                require_can_borrow(env, cache, account.spoke_id, &hub_asset);
            }
        }
    }
}

/// Enforces the leg's halt policy. Missing listings remain exitable and
/// seizable so delisting cannot strand positions or prevent liquidation.
pub(crate) fn enforce_spoke_asset_flags(
    env: &Env,
    cache: &mut Context,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
    freeze: FreezePolicy,
) {
    if let Some(sa) = cache.cached_spoke_asset(spoke_id, hub_asset) {
        match freeze {
            FreezePolicy::BlockOnEntry => {
                assert_with_error!(env, !sa.paused, SpokeError::SpokeAssetPaused);
                assert_with_error!(env, !sa.frozen, SpokeError::SpokeAssetFrozen);
            }
            FreezePolicy::AllowOnExit => {
                assert_with_error!(env, !sa.paused, SpokeError::SpokeAssetPaused);
            }
            FreezePolicy::SeizureLeg => {
                assert_with_error!(env, !sa.no_seize, SpokeError::SpokeAssetSeizureHalted);
            }
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
