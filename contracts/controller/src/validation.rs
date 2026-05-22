use common::constants::{BPS, MAX_FLASHLOAN_FEE_BPS, MAX_LIQUIDATION_BONUS, MIN_DUST_FLOOR_WAD};
use common::errors::{CollateralError, FlashLoanError, GenericError, OracleError};
use common::math::fp::Wad;
use common::types::{Account, AccountPositionType, AssetConfigRaw, MarketStatus, Payment};
use soroban_sdk::{panic_with_error, token, Address, Env, Map, Vec};

use crate::cache::ControllerCache;
use crate::storage::iter_typed_positions;
use crate::{helpers, storage};

// Unwraps Option or panics with InternalError.
#[inline]
pub fn expect_invariant<T>(env: &Env, opt: Option<T>) -> T {
    opt.unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError))
}

// Rejects unsupported assets.
pub fn require_asset_supported(env: &Env, cache: &mut ControllerCache, asset: &Address) {
    let _ = env;
    let _ = cache.cached_market_config(asset);
}

pub fn require_market_active(env: &Env, cache: &mut ControllerCache, asset: &Address) {
    let market = cache.cached_market_config(asset);
    if market.status != MarketStatus::Active {
        panic_with_error!(env, GenericError::PairNotActive);
    }
}

pub fn require_account_owner_match(env: &Env, account: &Account, caller: &Address) {
    if account.owner != *caller {
        panic_with_error!(env, GenericError::AccountNotInMarket);
    }
}

pub fn require_not_flash_loaning(env: &Env) {
    if storage::is_flash_loan_ongoing(env) {
        panic_with_error!(env, FlashLoanError::FlashLoanOngoing);
    }
}

pub fn require_amount_positive(env: &Env, amount: i128) {
    if amount <= 0 {
        panic_with_error!(env, GenericError::AmountMustBePositive);
    }
}

pub fn require_non_empty_payments<T>(env: &Env, payments: &Vec<T>) {
    if payments.is_empty() {
        panic_with_error!(env, GenericError::InvalidPayments);
    }
}

pub fn require_credit_not_above_sent(env: &Env, sent: i128, received: i128) {
    if received > sent {
        panic_with_error!(env, GenericError::InvalidPayments);
    }
}

pub fn require_healthy_account(env: &Env, cache: &mut ControllerCache, account: &Account) {
    if account.borrow_positions.is_empty() {
        return;
    }

    let hf = helpers::calculate_health_factor(
        env,
        cache,
        &account.supply_positions,
        &account.borrow_positions,
    );
    if hf < Wad::ONE {
        panic_with_error!(env, CollateralError::InsufficientCollateral);
    }
}

pub fn require_within_ltv(env: &Env, cache: &mut ControllerCache, account: &Account) {
    if account.borrow_positions.is_empty() {
        return;
    }

    let ltv_collateral_wad =
        helpers::calculate_ltv_collateral_wad(env, cache, &account.supply_positions).raw();

    let mut total_borrow_wad: i128 = 0;
    for (asset, position) in iter_typed_positions(&account.borrow_positions) {
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);
        let value = helpers::position_value(
            env,
            position.scaled_amount,
            market_index.borrow_index,
            feed.price,
        )
        .raw();
        total_borrow_wad = total_borrow_wad
            .checked_add(value)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    }

    if ltv_collateral_wad < total_borrow_wad {
        panic_with_error!(env, CollateralError::InsufficientCollateral);
    }
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
    for i in 0..assets.len() {
        let (asset, _) = expect_invariant(env, assets.get(i));
        if seen.contains_key(asset.clone()) {
            continue;
        }
        seen.set(asset.clone(), true);

        let already_present = match position_type {
            AccountPositionType::Deposit => account.supply_positions.contains_key(asset),
            AccountPositionType::Borrow => account.borrow_positions.contains_key(asset),
        };
        if !already_present {
            new_positions_count = new_positions_count
                .checked_add(1)
                .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
        }
    }

    let total_positions = current_count
        .checked_add(new_positions_count)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    if total_positions > max_allowed {
        panic_with_error!(env, CollateralError::PositionLimitExceeded);
    }
}

pub fn validate_risk_bounds(env: &Env, ltv: u32, threshold: u32, bonus: u32) {
    let ltv_i = i128::from(ltv);
    let threshold_i = i128::from(threshold);
    let bonus_i = i128::from(bonus);
    if threshold_i <= ltv_i || threshold_i > BPS {
        panic_with_error!(env, CollateralError::InvalidLiqThreshold);
    }
    if bonus_i > MAX_LIQUIDATION_BONUS {
        panic_with_error!(env, CollateralError::InvalidLiqThreshold);
    }
}

pub fn validate_and_fetch_token_decimals(env: &Env, token: &Address) -> u32 {
    let token_client = token::Client::new(env, token);
    let decimals = match token_client.try_decimals() {
        Ok(Ok(d)) => d,
        _ => panic_with_error!(env, GenericError::InvalidAsset),
    };
    if !matches!(token_client.try_symbol(), Ok(Ok(_))) {
        panic_with_error!(env, GenericError::InvalidAsset);
    }
    decimals
}

pub fn validate_asset_config(env: &Env, config: &AssetConfigRaw) {
    validate_risk_bounds(
        env,
        config.loan_to_value_bps,
        config.liquidation_threshold_bps,
        config.liquidation_bonus_bps,
    );

    if i128::from(config.liquidation_fees_bps) > BPS {
        panic_with_error!(env, CollateralError::InvalidLiqThreshold);
    }

    if config.supply_cap < 0 || config.borrow_cap < 0 {
        panic_with_error!(env, CollateralError::InvalidBorrowParams);
    }

    if config.isolation_debt_ceiling_usd_wad < 0 {
        panic_with_error!(env, CollateralError::InvalidBorrowParams);
    }

    if i128::from(config.flashloan_fee_bps) > MAX_FLASHLOAN_FEE_BPS {
        panic_with_error!(env, FlashLoanError::StrategyFeeExceeds);
    }

    let dust_disabled = config.min_collat_floor_usd_wad == 0 && config.min_debt_floor_usd_wad == 0;
    if !dust_disabled
        && (config.min_collat_floor_usd_wad < MIN_DUST_FLOOR_WAD
            || config.min_debt_floor_usd_wad < MIN_DUST_FLOOR_WAD)
    {
        panic_with_error!(env, CollateralError::DustFloorTooLow);
    }
}

// Validates oracle price bounds.
pub fn validate_oracle_bounds(env: &Env, first: i128, last: i128) {
    if last <= first {
        panic_with_error!(env, OracleError::BadAnchorTolerances);
    }
    // Upper bound on `last` is enforced by the caller's range check via
    // `MAX_LAST_TOLERANCE`.
}
