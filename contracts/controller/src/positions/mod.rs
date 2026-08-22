//! Shared position-accounting toolkit for the verb modules (supply, debt,
//! liquidation) and the strategies: the "leg" vocabulary (`LegOutcome`,
//! `merge_debt_leg`/`merge_withdraw_leg`, `apply_leg_usage`), the entry gates
//! (`require_can_supply`/`require_can_borrow`), and the persistence tail
//! (`finalize_position_flow`: persist spoke usage and positions, then emit
//! the position batch).

pub(crate) mod debt;

use common::errors::{CollateralError, GenericError, SpokeError};
use common::math::fp::Ray;
use common::types::{
    Account, AccountPosition, AccountPositionType, AggregatedPayments, AssetConfig, DebtPosition,
    HubAssetKey, HubPayment, MarketIndexRaw, PoolAction, PoolPositionMutation, ScaledPositionRaw,
};
use soroban_sdk::{assert_with_error, panic_with_error, Env, IntoVal, TryFromVal, Val, Vec};

use crate::account;
use crate::context::Cache;
use crate::events;
use crate::risk::{self, validation};
use crate::spoke_usage::UsageSide;
use crate::storage;

pub(crate) mod liquidation;
pub(crate) use debt::{
    apply_repay_batch, borrow_into_controller, execute_repayment, process_borrow, process_repay,
    RepaymentRequest,
};
pub(crate) use supply::{
    apply_withdraw_batch, execute_withdrawal, merge_withdraw_leg, process_supply, process_withdraw,
    WithdrawKind, WithdrawalRequest,
};
pub(crate) mod supply;

pub(crate) struct LegOutcome {
    pub new_scaled: Ray,
    pub market_index: MarketIndexRaw,
    pub amount: i128,
}

impl From<&PoolPositionMutation> for LegOutcome {
    /// Builds a `LegOutcome` from a pool position mutation's resulting scaled
    /// amount, market index, and settled amount.
    fn from(mutation: &PoolPositionMutation) -> Self {
        Self {
            new_scaled: Ray::from(mutation.position.scaled_amount),
            market_index: mutation.market_index.clone(),
            amount: mutation.actual_amount,
        }
    }
}

/// Pairs each request entry with its corresponding pool result by position
/// and invokes `f` on every pair. Panics with `GenericError::InternalError`
/// if `entries` and `results` differ in length.
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

/// Restamps supply position LTVs to their current listed values, then
/// asserts the account still clears the post-pool solvency gates (collateral
/// coverage, health factor, minimum borrow collateral). Returns whether the
/// LTV restamp changed any position.
pub(crate) fn enforce_post_pool_solvency(
    env: &Env,
    cache: &mut Cache,
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

/// Updates the spoke's usage accounting for one leg: on entry, applies the
/// scaled increase (capped, tracked with the leg's market index and asset
/// decimals); on exit, applies the scaled decrease.
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

/// Merges a debt leg's pool outcome into `account`'s position for
/// `hub_asset`. Uses the existing scaled amount as the baseline (zero when
/// opening a position on entry, or panics if none exists on exit), then
/// updates spoke usage, the cached market index, and the position-update
/// event before storing or removing the debt position.
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

/// Which listing halt flags a given leg honours.
///
/// The three policies are disjoint on purpose. `paused` is a *user-activity* halt and does not
/// reach the seizure leg, because seizure is pro-rata over an account's entire collateral set:
/// gating it on `paused` turns a per-listing halt into a protocol-wide liquidation halt for
/// every account holding that collateral. Seizure has its own flag instead. See ADR-0008.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum FreezePolicy {
    /// New exposure: rejects `paused` and `frozen`.
    BlockOnEntry,
    /// User-initiated exit: rejects `paused`, tolerates `frozen`.
    AllowOnExit,
    /// Liquidation seizure: rejects `no_seize` only, tolerates `paused` and `frozen`.
    SeizureLeg,
}

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

/// Writes the account's supply and/or debt position maps to storage as
/// selected by `sides`, renews the account's storage TTL if either was
/// written, and removes the account entry if `remove_if_empty` is set and the
/// account now holds no positions.
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

/// Persists spoke usage, writes the account's positions to storage, and
/// emits the batched position-update event; the common tail for every
/// supply, withdraw, borrow, and repay flow.
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

/// Asserts the hub is active, the asset is listed on an active spoke
/// (unlisted assets revert `AssetNotInSpoke`), and the spoke asset is neither
/// paused nor frozen (new entries: frozen blocks; paused blocks every verb).
/// Returns the asset config so the caller can apply its verb-specific
/// permission check.
fn require_listed_unhalted_config(
    env: &Env,
    cache: &mut Cache,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
) -> AssetConfig {
    cache.require_hub_active(hub_asset.hub_id);
    let asset_config = cache.require_listed_active_config(spoke_id, hub_asset);
    enforce_spoke_asset_flags(env, cache, spoke_id, hub_asset, FreezePolicy::BlockOnEntry);
    asset_config
}

/// Asserts the hub is active, the asset is listed on an active spoke, the
/// spoke asset is neither paused nor frozen, and the asset config permits
/// borrowing.
pub(crate) fn require_can_borrow(
    env: &Env,
    cache: &mut Cache,
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

/// Asserts the hub is active, the asset is listed on an active spoke, the
/// spoke asset is neither paused nor frozen, and the asset config permits
/// supply as collateral.
pub(crate) fn require_can_supply(
    env: &Env,
    cache: &mut Cache,
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

/// Checks the bulk position-count limit for `position_type`, then runs the
/// supply or borrow entry gates for every hub asset in `aggregated`.
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
                require_can_borrow(env, cache, account.spoke_id, &hub_asset);
            }
        }
    }
}

/// Asserts the spoke asset's halt flags permit this leg, per `freeze`.
///
/// No-op if the asset has no cached spoke config: a delisted asset must stay exitable and
/// seizable, or its holders would be stranded and unliquidatable.
pub(crate) fn enforce_spoke_asset_flags(
    env: &Env,
    cache: &mut Cache,
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

/// Builds a `PoolAction` from a position's scaled representation, the
/// amount, and the hub asset key.
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

/// Returns the account's supply position for `hub_asset`, panicking with
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

/// Returns the account's debt position for `hub_asset`, panicking with
/// `DebtPositionNotFound` if none exists.
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
