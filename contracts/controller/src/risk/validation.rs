//! Solvency and position-limit gates enforced on the controller's write
//! paths: flash-loan re-entrancy guard, post-operation collateral/health-
//! factor checks, and per-account position-count limits.

use crate::risk;
use crate::spec_hooks;
use common::errors::*;
use common::math::fp::Wad;
use common::types::{Account, AccountPositionType, AggregatedPayments, HubAssetKey};
use soroban_sdk::{assert_with_error, panic_with_error, Env, Map};

use crate::{context::Cache, storage};

/// Panics with `FlashLoanError::FlashLoanOngoing` if a flash loan is
/// currently in progress.
pub(crate) fn require_not_flash_loaning(env: &Env) {
    assert_with_error!(
        env,
        !storage::is_flash_loan_ongoing(env),
        FlashLoanError::FlashLoanOngoing
    );
}

/// Validates `account`'s solvency after a pool operation. Returns
/// immediately if the account has no debt. Otherwise recomputes the
/// account's risk totals and panics with
/// `CollateralError::InsufficientCollateral` if LTV-weighted collateral is
/// below total debt or the health factor is below one, and panics with
/// `CollateralError::MinBorrowCollateralNotMet` if a nonzero minimum-
/// borrow-collateral floor is configured and LTV-weighted collateral falls
/// short of it.
pub(crate) fn require_post_pool_risk_gates(env: &Env, cache: &mut Cache, account: &Account) {
    if account.debt_free() {
        return;
    }

    let totals = risk::calculate_account_risk_totals(
        env,
        cache,
        &account.supply_positions,
        &account.borrow_positions,
    );

    assert_with_error!(
        env,
        totals.ltv_collateral >= totals.total_debt,
        CollateralError::InsufficientCollateral
    );

    spec_hooks::solvency_gate_checked();

    assert_with_error!(
        env,
        totals.health_factor >= Wad::ONE,
        CollateralError::InsufficientCollateral
    );

    let floor = storage::get_min_borrow_collateral_usd_wad(env);
    if floor != 0 && totals.ltv_collateral.raw() < floor {
        panic_with_error!(env, CollateralError::MinBorrowCollateralNotMet);
    }
}

/// Validates that applying `aggregated` would not push `account`'s
/// position count (supply or borrow, per `position_type`) past the
/// configured limit. Counts only hub assets not already held as a
/// position, deduplicating repeated entries in `aggregated`. Panics with
/// `GenericError::MathOverflow` if the resulting count overflows, and
/// asserts with `CollateralError::PositionLimitExceeded` if it exceeds the
/// configured maximum.
pub(crate) fn validate_bulk_position_limits(
    env: &Env,
    account: &Account,
    position_type: AccountPositionType,
    aggregated: &AggregatedPayments,
) {
    let limits = storage::get_position_limits(env);

    let (current_count, max_allowed) = match position_type {
        AccountPositionType::Deposit => {
            (account.supply_positions.len(), limits.max_supply_positions)
        }
        AccountPositionType::Borrow => {
            (account.borrow_positions.len(), limits.max_borrow_positions)
        }
    };

    let mut seen: Map<HubAssetKey, bool> = Map::new(env);
    let mut new_positions_count: u32 = 0;
    for (hub_asset, _) in aggregated.iter() {
        if seen.contains_key(hub_asset.clone()) {
            continue;
        }
        seen.set(hub_asset.clone(), true);

        let already_present = match position_type {
            AccountPositionType::Deposit => account.supply_positions.contains_key(hub_asset),
            AccountPositionType::Borrow => account.borrow_positions.contains_key(hub_asset),
        };
        if !already_present {
            new_positions_count += 1;
        }
    }

    let total_positions = current_count
        .checked_add(new_positions_count)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    assert_with_error!(
        env,
        total_positions <= max_allowed,
        CollateralError::PositionLimitExceeded
    );
}

#[cfg(test)]
#[path = "../../tests/validation.rs"]
mod tests;
