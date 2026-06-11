//! Shared validation gates for account ownership, market status, health factor,
//! LTV, position limits, asset config bounds, token shape, and oracle ranges.

use common::constants::{BPS, MAX_FLASHLOAN_FEE_BPS, MIN_DUST_FLOOR_WAD};
use common::errors::{CollateralError, FlashLoanError, GenericError};
use common::math::fp::Wad;
use common::types::{Account, AccountPositionType, AssetConfigRaw, MarketStatus, Payment};
use soroban_sdk::{assert_with_error, panic_with_error, token, Address, Env, Map, Vec};

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
    assets: &Vec<Payment>,
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
    for (asset, _) in assets.iter() {
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

pub fn validate_risk_bounds(env: &Env, ltv: u32, threshold: u32, bonus: u32) {
    let ltv_i = i128::from(ltv);
    let threshold_i = i128::from(threshold);
    let bonus_i = i128::from(bonus);
    if threshold_i <= ltv_i || threshold_i > BPS {
        panic_with_error!(env, CollateralError::InvalidLiqThreshold);
    }
    // threshold * (1 + bonus) <= 100%: liquidation seizure can never exceed the
    // collateral backing a position, so the bonus can never mint bad debt.
    assert_with_error!(
        env,
        threshold_i * (BPS + bonus_i) <= BPS * BPS,
        CollateralError::InvalidLiqThreshold
    );
}

pub fn validate_and_fetch_token_decimals(env: &Env, token: &Address) -> u32 {
    let token_client = token::Client::new(env, token);
    let decimals = match token_client.try_decimals() {
        Ok(Ok(d)) => d,
        _ => panic_with_error!(env, GenericError::InvalidAsset),
    };
    assert_with_error!(
        env,
        matches!(token_client.try_symbol(), Ok(Ok(_))),
        GenericError::InvalidAsset
    );
    decimals
}

pub fn validate_asset_config(env: &Env, config: &AssetConfigRaw) {
    validate_risk_bounds(
        env,
        config.loan_to_value_bps,
        config.liquidation_threshold_bps,
        config.liquidation_bonus_bps,
    );

    assert_with_error!(
        env,
        i128::from(config.liquidation_fees_bps) <= BPS,
        CollateralError::InvalidLiqThreshold
    );

    if config.supply_cap < 0 || config.borrow_cap < 0 {
        panic_with_error!(env, CollateralError::InvalidBorrowParams);
    }

    assert_with_error!(
        env,
        config.isolation_debt_ceiling_usd_wad >= 0,
        CollateralError::InvalidBorrowParams
    );

    assert_with_error!(
        env,
        i128::from(config.flashloan_fee_bps) <= MAX_FLASHLOAN_FEE_BPS,
        FlashLoanError::StrategyFeeExceeds
    );

    let dust_disabled = config.min_collat_floor_usd_wad == 0 && config.min_debt_floor_usd_wad == 0;
    if !dust_disabled
        && (config.min_collat_floor_usd_wad < MIN_DUST_FLOOR_WAD
            || config.min_debt_floor_usd_wad < MIN_DUST_FLOOR_WAD)
    {
        panic_with_error!(env, CollateralError::DustFloorTooLow);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::constants::{MAX_FLASHLOAN_FEE_BPS, WAD};
    use common::types::{Account, AccountPositionType, AssetConfigRaw};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env, Vec};

    fn sample_asset_config(env: &Env) -> AssetConfigRaw {
        AssetConfigRaw {
            loan_to_value_bps: 7_500,
            liquidation_threshold_bps: 8_000,
            liquidation_bonus_bps: 500,
            liquidation_fees_bps: 100,
            is_collateralizable: true,
            is_borrowable: true,
            is_isolated_asset: false,
            is_siloed_borrowing: false,
            is_flashloanable: true,
            isolation_borrow_enabled: true,
            isolation_debt_ceiling_usd_wad: 1_000 * WAD,
            flashloan_fee_bps: 9,
            borrow_cap: 1_000_000,
            supply_cap: 5_000_000,
            min_collat_floor_usd_wad: 10 * WAD,
            min_debt_floor_usd_wad: 10 * WAD,
            e_mode_categories: Vec::new(env),
        }
    }

    #[test]
    #[should_panic]
    fn test_validate_risk_bounds_rejects_threshold_above_bps() {
        let env = Env::default();
        validate_risk_bounds(&env, 5_000, 10_001, 100);
    }

    #[test]
    #[should_panic]
    fn test_validate_asset_config_rejects_negative_supply_cap() {
        let env = Env::default();
        let mut cfg = sample_asset_config(&env);
        cfg.supply_cap = -1;
        validate_asset_config(&env, &cfg);
    }

    #[test]
    #[should_panic]
    fn test_validate_asset_config_rejects_dust_floor_below_minimum() {
        let env = Env::default();
        let mut cfg = sample_asset_config(&env);
        cfg.min_collat_floor_usd_wad = MIN_DUST_FLOOR_WAD - 1;
        validate_asset_config(&env, &cfg);
    }

    #[test]
    #[should_panic]
    fn test_validate_asset_config_rejects_flashloan_fee_above_cap() {
        let env = Env::default();
        let mut cfg = sample_asset_config(&env);
        cfg.flashloan_fee_bps = (MAX_FLASHLOAN_FEE_BPS + 1) as u32;
        validate_asset_config(&env, &cfg);
    }

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
            mode: common::types::PositionMode::Normal,
            is_isolated: false,
            isolated_asset: None,
        };
        let assets: Vec<(Address, i128)> = Vec::from_array(
            &env,
            [
                (asset.clone(), 100),
                (asset.clone(), 200),
            ],
        );
        env.as_contract(&contract_id, || {
            validate_bulk_position_limits(
                &env,
                &account,
                AccountPositionType::Deposit,
                &assets,
            );
        });
    }
}
