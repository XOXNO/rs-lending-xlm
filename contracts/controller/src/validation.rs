//! Shared validation gates for account ownership, market status, health factor,
//! LTV, and position limits.

use common::constants::POSITION_LIMIT_MAX;
use common::errors::{CollateralError, FlashLoanError, GenericError};
use common::math::fp::Wad;
use controller_interface::types::{
    Account, AccountPositionType, MarketStatus, Payment, PositionLimits,
};
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env, Map, Vec};

use crate::cache::Cache;
use crate::{helpers, storage};

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

pub use common::validation::{require_positive_amount, require_wasm_receiver};

pub fn require_non_empty_payments<T>(env: &Env, payments: &Vec<T>) {
    assert_with_error!(env, !payments.is_empty(), GenericError::InvalidPayments);
}

/// Post-pool LTV, health factor, and min-borrow-collateral gates in one
/// prefetch and one portfolio walk. No-op when the account is debt-free.
pub fn require_post_pool_risk_gates(env: &Env, cache: &mut Cache, account: &Account) {
    if account.borrow_positions.is_empty() {
        return;
    }

    let totals = helpers::calculate_post_pool_risk_totals(
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

    if totals.total_debt != Wad::ZERO {
        let hf = totals.weighted_collateral.div_floor(env, totals.total_debt);
        assert_with_error!(env, hf >= Wad::ONE, CollateralError::InsufficientCollateral);
    }

    let floor = storage::get_min_borrow_collateral_usd_wad(env);
    if floor != 0 && totals.ltv_collateral.raw() < floor {
        panic_with_error!(env, CollateralError::MinBorrowCollateralNotMet);
    }
}

/// Re-checks position-limit bounds at the controller boundary. Governance owns
/// the authoritative validation; this setter mirrors it so persisted limits
/// stay inside the budget-proven `1..=POSITION_LIMIT_MAX` envelope.
pub fn validate_position_limits(env: &Env, limits: &PositionLimits) {
    if limits.max_supply_positions == 0
        || limits.max_borrow_positions == 0
        || limits.max_supply_positions > POSITION_LIMIT_MAX
        || limits.max_borrow_positions > POSITION_LIMIT_MAX
    {
        panic_with_error!(env, GenericError::InvalidPositionLimits);
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
    use controller_interface::types::{Account, AccountPositionType};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env, Vec};

    #[test]
    fn test_validate_bulk_position_limits_dedupes_duplicate_assets() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let contract_id = env.register(Controller, (admin,));
        let asset = Address::generate(&env);
        let owner = Address::generate(&env);
        let account = Account {
            owner,
            supply_positions: Map::new(&env),
            borrow_positions: Map::new(&env),
            e_mode_category_id: 0,
            mode: controller_interface::types::PositionMode::Normal,
        };
        let assets: Vec<(Address, i128)> =
            Vec::from_array(&env, [(asset.clone(), 100), (asset.clone(), 200)]);
        env.as_contract(&contract_id, || {
            validate_bulk_position_limits(&env, &account, AccountPositionType::Deposit, &assets);
        });
    }
}
