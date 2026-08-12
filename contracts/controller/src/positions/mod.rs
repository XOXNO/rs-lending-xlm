//! Shared building blocks for the position-mutation flows implemented in the sibling
//! `borrow`, `repay`, `supply`, `withdraw`, and `liquidation` modules.
//!
//! Provides the caller/auth entry gate, per-leg pool-result merging for debt positions,
//! entry-gate validation for deposits and borrows, post-pool solvency enforcement, and the
//! account-state persistence and event-emission tail shared by every verb.

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

/// Reduced pool-call result for one position leg: the fields the merge step consumes,
/// independent of which pool call produced them.
pub(crate) struct LegOutcome {
    /// Post-call scaled shares, as owned by the pool.
    pub new_scaled: Ray,
    /// Market index snapshot returned by the pool call for this leg's asset.
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

/// Requires `caller`'s authorization signature and reverts if a flash loan is in progress.
pub(crate) fn require_position_caller(env: &Env, caller: &Address) {
    caller.require_auth();
    validation::require_not_flash_loaning(env);
}

/// Restamps live LTV on every listed supply leg, then runs the post-pool risk gates. Returns
/// whether the supply position map was modified.
pub(crate) fn enforce_post_pool_solvency(
    env: &Env,
    cache: &mut Cache,
    account: &mut Account,
) -> bool {
    let restamped = risk::restamp_listed_supply_ltv(cache, account);
    validation::require_post_pool_risk_gates(env, cache, account);
    restamped
}

/// Direction of a position leg. `Entry` increases the position and carries the asset decimals
/// used for cap enforcement; `Exit` decreases it.
#[derive(Clone, Copy)]
pub(crate) enum LegDirection {
    Entry { asset_decimals: u32 },
    Exit,
}

/// Applies a leg's scaled-share delta to spoke usage: computes `new - old` for `Entry` and
/// `old - new` for `Exit`, and enforces caps only for `Entry`.
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

/// Folds one debt-side pool result into the account, spoke usage, and events. Handles both
/// borrow (`Entry`) and repay (`Exit`) directions.
///
/// Panics with `DebtPositionNotFound` if `direction` is `Exit` and `hub_asset` has no debt
/// position on `account`.
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

/// Whether a frozen listing blocks the verb.
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

/// Persists `account`'s supply and/or debt position maps according to `sides`, renews the
/// account's storage TTL if either side is written, and runs empty-account cleanup when
/// `remove_if_empty` is set.
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
    if sides.supply || sides.debt {
        storage::renew_user_account(env, account_id);
    }
    if remove_if_empty {
        account::cleanup_account_if_empty(env, account, account_id);
    }
}

/// Persists buffered spoke usage, then `account`'s supply and/or debt position maps, via
/// [`persist_account_positions`].
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

/// Persists `account`'s position state (via [`persist_position_flow`]) and emits the
/// position-change events for `account_id`. `remove_if_empty` is `true` for user
/// withdraw and for strategy finalization (`strategy_finalize`); borrow, repay, and
/// supply pass `false`, leaving the account in storage even if a side ends up empty.
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

/// Validates hub-active status, spoke listing, pause/freeze flags (via
/// [`FreezePolicy::BlockOnEntry`]), and `can_supply` for a single asset. Equivalent to the
/// `Deposit` branch of [`validate_position_entry_gates`] without the bulk position-limit checks.
pub(crate) fn require_can_supply(
    env: &Env,
    cache: &mut Cache,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
) {
    cache.require_hub_active(hub_asset.hub_id);
    // Unlisted assets revert `AssetNotInSpoke`.
    let asset_config = cache.require_listed_active_config(spoke_id, hub_asset);
    // New entries: frozen blocks; paused blocks every verb.
    enforce_spoke_asset_flags(env, cache, spoke_id, hub_asset, FreezePolicy::BlockOnEntry);
    assert_with_error!(
        env,
        asset_config.can_supply(),
        CollateralError::NotCollateral
    );
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
        match position_type {
            AccountPositionType::Deposit => {
                require_can_supply(env, cache, account.spoke_id, &hub_asset);
            }
            AccountPositionType::Borrow => {
                cache.require_hub_active(hub_asset.hub_id);
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
                assert_with_error!(
                    env,
                    asset_config.can_borrow(),
                    CollateralError::AssetNotBorrowable
                );
            }
        }
    }
}

/// Enforces per-spoke paused/frozen flags when the asset is listed in the cache. Reverts if
/// paused, for every verb. Reverts if frozen only under [`FreezePolicy::BlockOnEntry`]. Does
/// nothing if the asset has no cached spoke listing.
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

/// Builds a [`PoolAction`] from a scaled position, amount, and hub asset.
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

/// Looks up the supply position for `hub_asset` on `account`. Panics with
/// `CollateralPositionNotFound` if none exists.
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

/// Looks up the debt position for `hub_asset` on `account`. Panics with `DebtPositionNotFound`
/// if none exists.
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
