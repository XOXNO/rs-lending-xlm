//! Shared validation gates for account ownership, market status, health factor,
//! LTV, and position limits.

use common::errors::*;
use common::math::fp::Wad;
pub use common::validation::{require_positive_amount, require_wasm_receiver};
use controller_interface::types::{Account, AccountPositionType, MarketStatus, Payment};
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env, Map, Vec};

use crate::{cache::Cache, helpers, storage};

/// Unwraps a controller-built value or panics with `InternalError`.
///
/// Missing values here indicate corrupted storage or a caller logic bug after
/// prior length/key checks, not malformed user input.
#[inline]
pub fn expect_invariant<T>(env: &Env, opt: Option<T>) -> T {
    opt.unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError))
}

/// Panics with the market-config error when `asset` has no market; the cache
/// entry populated by the read is the "supported" signal callers rely on.
pub fn require_asset_supported(_env: &Env, cache: &mut Cache, asset: &Address) {
    cache.cached_market_config(asset);
}

pub fn require_market_active(env: &Env, cache: &mut Cache, asset: &Address) {
    let market = cache.cached_market_config(asset);
    assert_with_error!(
        env,
        market.status == MarketStatus::Active,
        GenericError::PairNotActive
    );
}

pub fn require_account_owner_match(env: &Env, account: &Account, caller: &Address) {
    assert_with_error!(
        env,
        account.owner == *caller,
        GenericError::AccountNotInMarket
    );
}

pub fn require_not_flash_loaning(env: &Env) {
    assert_with_error!(
        env,
        !storage::is_flash_loan_ongoing(env),
        FlashLoanError::FlashLoanOngoing
    );
}

pub fn require_non_empty_payments<T>(env: &Env, payments: &Vec<T>) {
    assert_with_error!(env, !payments.is_empty(), GenericError::InvalidPayments);
}

/// Post-pool LTV, health factor, and min-borrow-collateral gates in one
/// prefetch and one portfolio walk. No-op when the account is debt-free.
pub fn require_post_pool_risk_gates(env: &Env, cache: &mut Cache, account: &Account) {
    if account.borrow_positions.is_empty() {
        return;
    }

    let totals = helpers::calculate_account_risk_totals(
        env,
        cache,
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

    // Debt-free accounts carry a saturated health factor, so this passes
    // without a special case.
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

pub fn validate_bulk_position_limits(
    env: &Env,
    account: &Account,
    position_type: AccountPositionType,
    aggregated: &Vec<Payment>,
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

    let mut seen: Map<Address, bool> = Map::new(env);
    let mut new_positions_count: u32 = 0;
    for (asset, _) in aggregated.iter() {
        if seen.contains_key(asset.clone()) {
            continue;
        }
        seen.set(asset.clone(), true);

        let already_present = match position_type {
            AccountPositionType::Deposit => account.supply_positions.contains_key(asset),
            AccountPositionType::Borrow => account.borrow_positions.contains_key(asset),
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
mod tests {
    use super::*;
    use crate::Controller;
    use common::types::pool::{AccountPositionRaw, DebtPositionRaw};
    use controller_interface::types::{Account, AccountPositionType, PositionLimits, PositionMode};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env, Vec};

    fn new_controller(env: &Env) -> Address {
        let admin = Address::generate(env);
        env.register(Controller, (admin,))
    }

    /// Account holding at most one existing supply and/or borrow position. Values
    /// are placeholders; the guard reads only key presence.
    fn account_with(env: &Env, supply: Option<&Address>, borrow: Option<&Address>) -> Account {
        let mut supply_positions = Map::new(env);
        if let Some(asset) = supply {
            supply_positions.set(
                asset.clone(),
                AccountPositionRaw {
                    scaled_amount_ray: 1,
                    liquidation_threshold_bps: 0,
                    liquidation_bonus_bps: 0,
                    loan_to_value_bps: 0,
                },
            );
        }
        let mut borrow_positions = Map::new(env);
        if let Some(asset) = borrow {
            borrow_positions.set(
                asset.clone(),
                DebtPositionRaw {
                    scaled_amount_ray: 1,
                },
            );
        }
        Account {
            owner: Address::generate(env),
            supply_positions,
            borrow_positions,
            e_mode_category_id: 0,
            mode: PositionMode::Normal,
        }
    }

    /// Writes the limits and runs `f` inside the controller's storage context;
    /// both the setter and the guard read instance storage.
    fn with_limits(
        env: &Env,
        contract: &Address,
        max_supply: u32,
        max_borrow: u32,
        f: impl FnOnce(),
    ) {
        env.as_contract(contract, || {
            storage::set_position_limits(
                env,
                &PositionLimits {
                    max_supply_positions: max_supply,
                    max_borrow_positions: max_borrow,
                },
            );
            f();
        });
    }

    #[test]
    fn test_validate_bulk_position_limits_dedupes_duplicate_assets() {
        let env = Env::default();
        let contract = new_controller(&env);
        let asset = Address::generate(&env);
        let account = account_with(&env, None, None);
        // Same asset twice is one new position (1 <= cap 2).
        let aggregated =
            Vec::from_array(&env, [(asset.clone(), 100i128), (asset.clone(), 200i128)]);
        with_limits(&env, &contract, 2, 2, || {
            validate_bulk_position_limits(
                &env,
                &account,
                AccountPositionType::Deposit,
                &aggregated,
            );
        });
    }

    #[test]
    fn test_validate_bulk_position_limits_deposit_at_cap_with_existing_passes() {
        let env = Env::default();
        let contract = new_controller(&env);
        let existing = Address::generate(&env);
        let fresh = Address::generate(&env);
        let account = account_with(&env, Some(&existing), None);
        // `existing` is already supplied (not new); `fresh` is the 2nd -> 2 == cap.
        let aggregated = Vec::from_array(
            &env,
            [(existing.clone(), 100i128), (fresh.clone(), 100i128)],
        );
        with_limits(&env, &contract, 2, 0, || {
            validate_bulk_position_limits(
                &env,
                &account,
                AccountPositionType::Deposit,
                &aggregated,
            );
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #109)")]
    fn test_validate_bulk_position_limits_deposit_over_cap_panics() {
        let env = Env::default();
        let contract = new_controller(&env);
        let existing = Address::generate(&env);
        let a = Address::generate(&env);
        let b = Address::generate(&env);
        let account = account_with(&env, Some(&existing), None);
        // 1 existing + 2 new = 3 > cap 2.
        let aggregated = Vec::from_array(&env, [(a.clone(), 100i128), (b.clone(), 100i128)]);
        with_limits(&env, &contract, 2, 0, || {
            validate_bulk_position_limits(
                &env,
                &account,
                AccountPositionType::Deposit,
                &aggregated,
            );
        });
    }

    #[test]
    fn test_validate_bulk_position_limits_borrow_at_cap_with_existing_passes() {
        let env = Env::default();
        let contract = new_controller(&env);
        let existing = Address::generate(&env);
        let account = account_with(&env, None, Some(&existing));
        // Re-borrowing an existing asset adds no new position (1 == cap 1).
        let aggregated = Vec::from_array(&env, [(existing.clone(), 100i128)]);
        with_limits(&env, &contract, 0, 1, || {
            validate_bulk_position_limits(&env, &account, AccountPositionType::Borrow, &aggregated);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #109)")]
    fn test_validate_bulk_position_limits_borrow_over_cap_panics() {
        let env = Env::default();
        let contract = new_controller(&env);
        let a = Address::generate(&env);
        let b = Address::generate(&env);
        let account = account_with(&env, None, None);
        // 2 new borrows > cap 1; exercises the Borrow branch.
        let aggregated = Vec::from_array(&env, [(a.clone(), 100i128), (b.clone(), 100i128)]);
        with_limits(&env, &contract, 0, 1, || {
            validate_bulk_position_limits(&env, &account, AccountPositionType::Borrow, &aggregated);
        });
    }

    #[test]
    fn test_validate_bulk_position_limits_empty_aggregated_is_noop_at_cap() {
        let env = Env::default();
        let contract = new_controller(&env);
        let existing = Address::generate(&env);
        let account = account_with(&env, Some(&existing), None);
        // No new positions; current count (1) == cap (1) still passes.
        let aggregated: Vec<Payment> = Vec::new(&env);
        with_limits(&env, &contract, 1, 1, || {
            validate_bulk_position_limits(
                &env,
                &account,
                AccountPositionType::Deposit,
                &aggregated,
            );
        });
    }
}
