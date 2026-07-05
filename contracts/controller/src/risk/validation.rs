//! Shared account, market, risk, and position-limit gates.

use crate::risk;
use common::errors::*;
use common::math::fp::Wad;
use common::types::{Account, AccountPositionType, HubAssetKey};
pub use common::validation::{require_positive_amount, require_wasm_receiver};
use soroban_sdk::{assert_with_error, panic_with_error, Env, Map, Vec};

use crate::positions::AggregatedPayments;

use crate::{context::Cache, storage};

/// Hub-active gate for position flows.
pub(crate) use crate::config::require_hub_active;

/// Unwraps a controller-built value or panics with `InternalError`.
/// Missing values indicate corrupted storage or caller logic bugs after checks.
#[inline]
pub fn expect_invariant<T>(env: &Env, opt: Option<T>) -> T {
    opt.unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError))
}

/// Requires token-rooted oracle presence; pool and spoke gates follow.
pub fn require_market_active(env: &Env, cache: &mut Cache, hub_asset: &HubAssetKey) {
    assert_with_error!(
        env,
        cache.asset_oracle_exists(&hub_asset.asset),
        GenericError::PairNotActive
    );
}

/// Rejects the call while a flash loan is in progress.
pub fn require_not_flash_loaning(env: &Env) {
    assert_with_error!(
        env,
        !storage::is_flash_loan_ongoing(env),
        FlashLoanError::FlashLoanOngoing
    );
}

/// Rejects an empty payments vector.
pub fn require_non_empty_payments<T>(env: &Env, payments: &Vec<T>) {
    assert_with_error!(env, !payments.is_empty(), GenericError::InvalidPayments);
}

/// Post-pool LTV, health factor, and min-borrow-collateral gates in one
/// prefetch and one portfolio walk. No-op when the account is debt-free.
pub fn require_post_pool_risk_gates(env: &Env, cache: &mut Cache, account: &Account) {
    if account.borrow_positions.is_empty() {
        return;
    }

    let totals = risk::calculate_account_risk_totals(
        env,
        cache,
        account.spoke_id,
        &account.supply_positions,
        &account.borrow_positions,
    );

    assert_with_error!(
        env,
        totals.ltv_collateral.raw() >= totals.total_debt.raw(),
        CollateralError::InsufficientCollateral
    );

    // Mark that the solvency gate executed its collateral-covers-debt check.
    // Read only by the Blend-style "health-gated" Certora rules.
    #[cfg(feature = "certora")]
    crate::spec::health_ghost::set_checked();

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

/// Rejects a batch that would push supply/borrow position counts past their limits.
pub fn validate_bulk_position_limits(
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
