//! Core position money path: supply, borrow, withdraw, repay, liquidation.
//!
//! Auth and risk gates live on public entrypoints; pool settles shares/cash.
//!
//! # The four verbs are a 2×2
//!
//! |  | entry (grow) | exit (shrink) |
//! |---|---|---|
//! | **supply side** | [`supply`] | [`withdraw`] |
//! | **debt side** | [`borrow`] | [`repay`] |
//!
//! Almost every difference between the four modules follows from those two bits,
//! so read a module by asking which cell it is in:
//!
//! * **side** picks the position map, the [`crate::spoke::UsageSide`], the
//!   event recorder, and the position type. Only the supply side carries
//!   controller-owned risk params; debt positions are wholly pool-owned.
//! * **direction** picks `apply_spoke_entry` (which also enforces caps) versus
//!   `apply_spoke_exit`, and the sign of the scaled-share delta.
//! * **tokens move toward the pool** on supply·entry and debt·exit, so exactly
//!   [`supply`] and [`repay`] pre-transfer before the pool call.
//! * **the op can worsen health** on debt·entry and supply·exit, so exactly
//!   [`borrow`] and [`withdraw`] re-run post-pool solvency.
//!
//! # Stage ladder
//!
//! `process_*` → `settle_*` → build → `apply_*_batch` → merge per leg →
//! [`finalize_position_flow`]. `apply_*_batch` owns the one cross-contract call;
//! [`withdraw`] and [`repay`] expose theirs because liquidation and the strategy
//! legs enter at that depth (see [`withdraw`]'s tier table).
//!
//! Shared: [`require_position_caller`], [`for_each_leg`], [`apply_leg_usage`]
//! (owns the delta's sign), [`merge_debt_leg`] for both debt cells,
//! [`enforce_post_pool_solvency`], [`finalize_position_flow`].
//!
//! Not shared, on purpose: the supply-side merges. [`supply`] stamps risk params
//! before the new shares land, [`withdraw`] after, so each prices the min-HF gate
//! against the smaller balance. Merging them would need a flag for where the
//! stamp goes.

use common::errors::{CollateralError, GenericError, SpokeError};
use common::math::fp::Ray;
use common::types::{
    Account, AccountPosition, AccountPositionType, AggregatedPayments, DebtPosition, HubAssetKey,
    HubPayment, MarketIndexRaw, PoolAction, PoolPositionMutation, ScaledPositionRaw,
};
use soroban_sdk::{
    assert_with_error, panic_with_error, Address, Env, IntoVal, TryFromVal, Val, Vec,
};

use crate::account;
use crate::config;
use crate::context::Cache;
use crate::events;
use crate::risk::{self, validation};
use crate::spoke::UsageSide;
use crate::storage;

pub(crate) mod borrow;
pub(crate) mod liquidation;
pub(crate) mod repay;
pub(crate) mod supply;
pub(crate) mod withdraw;

/// What the pool returned for one position leg, reduced to what the merge step
/// actually consumes.
///
/// Keeps the merges independent of which pool call produced the numbers, so the
/// batch paths and the net-settle path share one merge per side instead of
/// hand-rolling their own tail.
pub(crate) struct LegOutcome {
    /// Post-call scaled shares, as owned by the pool.
    pub new_scaled: Ray,
    pub market_index: MarketIndexRaw,
    /// Asset-native amount moved, for the event payload.
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

/// Walks one pool call's entries against its results, in input order.
///
/// One assert beats a per-index `results.get(i)`, which tolerates a long return.
///
/// # Errors
/// * [`GenericError::InternalError`] - result count differs from entry count.
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

/// Entry gate for every public verb: caller signed, not inside a flash loan.
pub(crate) fn require_position_caller(env: &Env, caller: &Address) {
    caller.require_auth();
    validation::require_not_flash_loaning(env);
}

/// Restamps live LTV on every listed supply leg, then runs the post-pool gates.
///
/// Only the health-reducing cells need it: debt·entry and supply·exit. Covers all
/// listed legs, not just touched ones, since the gates read live config. Returns
/// whether the supply map was dirtied, so the caller knows to persist that side.
pub(crate) fn enforce_post_pool_solvency(
    env: &Env,
    cache: &mut Cache,
    account: &mut Account,
) -> bool {
    let restamped = risk::restamp_listed_supply_ltv(cache, account);
    validation::require_post_pool_risk_gates(env, cache, account);
    restamped
}

/// Which half of the 2×2 a leg is on. `Entry` grows and carries the asset
/// decimals cap enforcement needs; `Exit` shrinks and needs nothing extra.
#[derive(Clone, Copy)]
pub(crate) enum LegDirection {
    Entry { asset_decimals: u32 },
    Exit,
}

/// Applies a leg's scaled-share delta to spoke usage. Owns the sign: entry is
/// `new - old`, exit is `old - new`. Only entry enforces caps.
pub(crate) fn apply_leg_usage(
    env: &Env,
    cache: &mut Cache,
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

/// Folds one debt-side pool result into the account, spoke usage and events.
///
/// Serves borrow (`Entry`) and repay (`Exit`): debt is wholly pool-owned, so there
/// are no risk params to stamp and both directions share one body.
pub(crate) fn merge_debt_leg(
    env: &Env,
    account: &mut Account,
    action: events::PositionAction,
    hub_asset: &HubAssetKey,
    direction: LegDirection,
    outcome: &LegOutcome,
    cache: &mut Cache,
) {
    // Entry may open a leg; exit must find one.
    let old_scaled = match direction {
        LegDirection::Entry { .. } => account
            .borrow_positions
            .get(hub_asset.clone())
            .map_or(Ray::ZERO, |p| Ray::from(p.scaled_amount)),
        LegDirection::Exit => get_debt_position_or_panic(env, account, hub_asset).scaled_amount,
    };
    let position = DebtPosition {
        scaled_amount: outcome.new_scaled,
    };

    apply_leg_usage(
        env,
        cache,
        account.spoke_id,
        UsageSide::Borrow,
        hub_asset,
        direction,
        old_scaled,
        outcome,
    );
    cache.put_market_index(hub_asset, &outcome.market_index);
    cache.record_debt_position_update(
        action,
        hub_asset,
        outcome.market_index.borrow_index,
        outcome.amount,
        &position,
    );
    account::update_or_remove_debt_position(account, hub_asset, &position);
}

/// Whether a frozen listing blocks the verb. Freeze must never block shrinking
/// an existing position, or governance could trap funds.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum FreezePolicy {
    /// New deposits and borrows: a frozen listing reverts.
    BlockOnEntry,
    /// Withdraw, repay, and net settle: a frozen listing still permits the exit.
    AllowOnExit,
}

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

/// Persist half of the position-flow tail: buffered spoke usage first, then the
/// account's position maps.
///
/// Split out from [`finalize_position_flow`] so liquidation, which must run its
/// bad-debt check between persisting and emitting, shares this ordering instead
/// of open-coding it.
pub(crate) fn persist_position_flow(
    env: &Env,
    account_id: u64,
    account: &Account,
    cache: &mut Cache,
    sides: PositionSides,
    remove_if_empty: bool,
) {
    cache.persist_spoke_usage();
    persist_account_positions(env, account_id, account, sides, remove_if_empty);
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
    persist_position_flow(env, account_id, account, cache, sides, remove_if_empty);
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
        // TODO: Use Cache to cache hub ID storage and avoid a loop of reads if that ID was fetched and status checked once
        config::require_hub_active(env, hub_asset.hub_id);
        // Unlisted assets revert `AssetNotInSpoke`.
        let asset_config = cache.require_listed_active_config(account.spoke_id, &hub_asset);
        // New entries: frozen blocks; paused blocks every verb.
        enforce_spoke_asset_flags(
            env,
            cache,
            account.spoke_id,
            &hub_asset,
            FreezePolicy::BlockOnEntry,
        );
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
/// Paused always reverts, for every verb. Frozen reverts only under
/// [`FreezePolicy::BlockOnEntry`]. Missing listing is a no-op here (callers that
/// need a listing use `require_listed_active_config` first), so a delisted asset
/// stays exitable.
pub(crate) fn enforce_spoke_asset_flags(
    env: &Env,
    cache: &mut Cache,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
    freeze: FreezePolicy,
) {
    if let Some(sa) = cache.cached_spoke_asset(spoke_id, hub_asset) {
        assert_with_error!(env, !sa.paused, SpokeError::SpokeAssetPaused);
        if freeze == FreezePolicy::BlockOnEntry {
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
