//! Shared validation gates for account ownership, market status, health factor,
//! LTV, and position limits.

use common::errors::{CollateralError, FlashLoanError, GenericError};
use common::math::fp::Wad;
use controller_interface::types::{Account, AccountPositionType, MarketStatus, Payment};
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

pub fn require_healthy_account(env: &Env, cache: &mut Cache, account: &Account) {
    if account.borrow_positions.is_empty() {
        return;
    }

    let hf = helpers::calculate_health_factor(
        env,
        cache,
        &account.supply_positions,
        &account.borrow_positions,
    );
    assert_with_error!(env, hf >= Wad::ONE, CollateralError::InsufficientCollateral);
}

pub fn require_within_ltv(env: &Env, cache: &mut Cache, account: &Account) {
    if account.borrow_positions.is_empty() {
        return;
    }

    // The gate owns its data: prefetch RedStone feeds and market indexes for
    // the union so the supply and debt valuations below share one bulk call
    // per side.
    let mut index_assets = account.supply_positions.keys();
    index_assets.append(&account.borrow_positions.keys());
    crate::oracle::prefetch_redstone_feeds(cache, &index_assets);
    cache.prefetch_market_indexes(&index_assets);

    let ltv_collateral_wad =
        helpers::calculate_ltv_collateral_wad(env, cache, &account.supply_positions).raw();
    let total_borrow_wad =
        helpers::calculate_total_debt_wad(env, cache, &account.borrow_positions).raw();

    assert_with_error!(
        env,
        ltv_collateral_wad >= total_borrow_wad,
        CollateralError::InsufficientCollateral
    );
}

pub fn validate_bulk_position_limits(
    env: &Env,
    account: &Account,
    position_type: AccountPositionType,
    plan: &Vec<Payment>,
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
    for (asset, _) in plan.iter() {
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
    use controller_interface::types::{Account, AccountPositionType};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env, Vec};

    #[test]
    fn test_validate_bulk_position_limits_dedupes_duplicate_assets() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let contract_id = env.register(crate::Controller, (admin,));
        let asset = Address::generate(&env);
        let owner = Address::generate(&env);
        let account = Account {
            owner,
            supply_positions: Map::new(&env),
            borrow_positions: Map::new(&env),
            e_mode_category_id: 0,
            mode: controller_interface::types::PositionMode::Normal,
            is_isolated: false,
            isolated_asset: None,
        };
        let assets: Vec<(Address, i128)> =
            Vec::from_array(&env, [(asset.clone(), 100), (asset.clone(), 200)]);
        env.as_contract(&contract_id, || {
            validate_bulk_position_limits(&env, &account, AccountPositionType::Deposit, &assets);
        });
    }
}
