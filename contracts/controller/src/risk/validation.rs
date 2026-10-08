use crate::risk;
use crate::spec_hooks;
use common::constants::{
    MIN_BORROWABLE_ASSET_DECIMALS, MIN_WHOLE_UNIT_COLLATERAL, POSITION_LIMIT_MAX,
};
use common::errors::*;
use common::math::fp::Wad;
use common::types::{Account, AccountPositionType, AggregatedPayments, HubAssetKey};
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env, Map, Vec};

use crate::storage::iter_typed_positions;
use crate::{context::Context, storage};

/// Authenticates `caller` and rejects execution during a flash loan.
pub(crate) fn require_authorized_caller(env: &Env, caller: &Address) {
    caller.require_auth();
    require_not_flash_loaning(env);
}

/// Rejects execution while the temporary flash-loan flag is set.
pub(crate) fn require_not_flash_loaning(env: &Env) {
    assert_with_error!(
        env,
        !storage::is_flash_loan_ongoing(env),
        FlashLoanError::FlashLoanOngoing
    );
}

/// Requires debt coverage by LTV-weighted collateral, health factor >= 1, and
/// the configured collateral floor. Debt-free accounts skip all three checks.
pub(crate) fn require_post_pool_risk_gates(env: &Env, cache: &mut Context, account: &Account) {
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

    spec_hooks::solvency_gate_checked(account);

    assert_with_error!(
        env,
        totals.health_factor >= Wad::ONE,
        CollateralError::InsufficientCollateral
    );

    let floor = storage::get_min_borrow_collateral_usd_wad(env);
    if floor != 0 && totals.ltv_collateral.raw() < floor {
        panic_with_error!(env, CollateralError::MinBorrowCollateralNotMet);
    }

    require_whole_unit_collateral_floor(env, cache, account);
}

/// Checks position limits only when adding a new hub-asset slot.
/// Deduplicates new assets and excludes positions already held by the account.
pub(crate) fn validate_bulk_position_limits(
    env: &Env,
    account: &Account,
    position_type: AccountPositionType,
    aggregated: &AggregatedPayments,
) {
    let limits = storage::get_position_limits(env);
    let max_allowed = match position_type {
        AccountPositionType::Deposit => limits.max_supply_positions,
        AccountPositionType::Borrow => limits.max_borrow_positions,
    };
    require_new_slots_within(env, account, position_type, aggregated, max_allowed);
}

/// Checks a Credit-mode liquidation receiver's new supply slots against the
/// supply limit, raised to the liquidated account's supply leg count capped at
/// `POSITION_LIMIT_MAX`. A limit lowered below that count still admits a
/// `Credit(0)` receiver for every seized leg.
pub(crate) fn validate_credit_receiver_position_limit(
    env: &Env,
    receiver: &Account,
    aggregated: &AggregatedPayments,
    liquidated_supply_legs: u32,
) {
    let max_allowed = storage::get_position_limits(env)
        .max_supply_positions
        .max(liquidated_supply_legs.min(POSITION_LIMIT_MAX));
    require_new_slots_within(
        env,
        receiver,
        AccountPositionType::Deposit,
        aggregated,
        max_allowed,
    );
}

/// Requires the held positions plus the distinct new slots in `aggregated` to
/// stay within `max_allowed`. Adds no check when no new slot opens.
fn require_new_slots_within(
    env: &Env,
    account: &Account,
    position_type: AccountPositionType,
    aggregated: &AggregatedPayments,
    max_allowed: u32,
) {
    let current_count = match position_type {
        AccountPositionType::Deposit => account.supply_positions.len(),
        AccountPositionType::Borrow => account.borrow_positions.len(),
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

    // Existing positions remain usable after governance lowers the position limit.
    if new_positions_count == 0 {
        return;
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

/// Rejects a new supply slot that would leave a collateral leg below
/// `MIN_BORROWABLE_ASSET_DECIMALS` beside any other supply position. An account
/// already holding two positions holds no such leg, so only the new assets and
/// a sole existing one are read.
pub(crate) fn require_whole_unit_isolation(
    env: &Env,
    cache: &mut Context,
    account: &Account,
    aggregated: &AggregatedPayments,
) {
    let existing = account.supply_positions.keys();
    let mut checked: Vec<HubAssetKey> = Vec::new(env);
    for (hub_asset, _) in aggregated.iter() {
        if !existing.contains(&hub_asset) && !checked.contains(&hub_asset) {
            checked.push_back(hub_asset);
        }
    }
    if checked.is_empty() || existing.len() + checked.len() <= 1 {
        return;
    }
    if existing.len() == 1 {
        checked.push_back(existing.get_unchecked(0));
    }
    for hub_asset in checked.iter() {
        let decimals = cache
            .cached_pool_sync_data(&hub_asset)
            .params
            .asset_decimals;
        assert_with_error!(
            env,
            decimals >= MIN_BORROWABLE_ASSET_DECIMALS,
            CollateralError::PositionLimitExceeded
        );
    }
}

/// Requires `MIN_WHOLE_UNIT_COLLATERAL` whole units in every supply leg below
/// `MIN_BORROWABLE_ASSET_DECIMALS`. Reads prices and indexes the solvency
/// check already loaded.
fn require_whole_unit_collateral_floor(env: &Env, cache: &mut Context, account: &Account) {
    for (hub_asset, position) in iter_typed_positions(&account.supply_positions) {
        let decimals = cache.cached_price(&hub_asset.asset).asset_decimals;
        if decimals >= MIN_BORROWABLE_ASSET_DECIMALS {
            continue;
        }
        let supply_index = cache.cached_market_index(&hub_asset).supply_index;
        let units = position
            .scaled_amount
            .mul(env, supply_index)
            .to_asset_floor(env, decimals);
        assert_with_error!(
            env,
            units >= MIN_WHOLE_UNIT_COLLATERAL,
            CollateralError::MinBorrowCollateralNotMet
        );
    }
}

#[cfg(test)]
#[path = "../../tests/validation.rs"]
mod tests;
