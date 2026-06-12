//! Risk-bound, asset-config, position-limit, and market-creation validation,
//! plus the live token-shape probe.

use common::constants::{BPS, MAX_FLASHLOAN_FEE_BPS, MIN_DUST_FLOOR_WAD};
use common::errors::{CollateralError, FlashLoanError, GenericError};
use common::types::MarketParamsRaw;
use controller_interface::types::{AssetConfigRaw, PositionLimits};
use soroban_sdk::{assert_with_error, panic_with_error, token, Address, Env};

// Supported SAC decimal range for RAY/WAD conversions.
const MIN_ASSET_DECIMALS: u32 = 1;
const MAX_ASSET_DECIMALS: u32 = 18;

/// Max supply/borrow positions configurable per account.
const POSITION_LIMIT_MAX: u32 = 10;

pub(crate) fn validate_risk_bounds(env: &Env, ltv: u32, threshold: u32, bonus: u32) {
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

pub(crate) fn validate_and_fetch_token_decimals(env: &Env, token: &Address) -> u32 {
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

pub(crate) fn validate_asset_config(env: &Env, config: &AssetConfigRaw) {
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

pub(crate) fn validate_position_limits(env: &Env, limits: &PositionLimits) {
    if limits.max_supply_positions == 0
        || limits.max_borrow_positions == 0
        || limits.max_supply_positions > POSITION_LIMIT_MAX
        || limits.max_borrow_positions > POSITION_LIMIT_MAX
    {
        panic_with_error!(env, GenericError::InvalidPositionLimits);
    }
}

pub(crate) fn validate_market_creation(
    env: &Env,
    asset: &Address,
    params: &MarketParamsRaw,
    config: &AssetConfigRaw,
    _token_decimals: u32,
) {
    assert_with_error!(env, params.asset_id == *asset, GenericError::WrongToken);
    #[cfg(not(feature = "testing"))]
    assert_with_error!(
        env,
        params.asset_decimals == _token_decimals,
        GenericError::InvalidAsset
    );

    assert_with_error!(
        env,
        (MIN_ASSET_DECIMALS..=MAX_ASSET_DECIMALS).contains(&params.asset_decimals),
        GenericError::InvalidAsset
    );

    validate_asset_config(env, config);
    params.verify_rate_model(env);
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::constants::{RAY, WAD};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Env, Vec};

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

    fn sample_market_params(asset: &Address, decimals: u32) -> MarketParamsRaw {
        MarketParamsRaw {
            max_borrow_rate_ray: RAY,
            base_borrow_rate_ray: 0,
            slope1_ray: RAY / 100,
            slope2_ray: RAY / 10,
            slope3_ray: RAY / 2,
            mid_utilization_ray: RAY / 2,
            optimal_utilization_ray: 8 * RAY / 10,
            max_utilization_ray: 95 * RAY / 100,
            reserve_factor_bps: 1_000,
            asset_id: asset.clone(),
            asset_decimals: decimals,
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
    #[should_panic]
    fn test_validate_position_limits_rejects_zero() {
        let env = Env::default();
        validate_position_limits(
            &env,
            &PositionLimits {
                max_supply_positions: 0,
                max_borrow_positions: 1,
            },
        );
    }

    #[test]
    #[should_panic]
    fn test_validate_position_limits_rejects_above_cap() {
        let env = Env::default();
        validate_position_limits(
            &env,
            &PositionLimits {
                max_supply_positions: 1,
                max_borrow_positions: POSITION_LIMIT_MAX + 1,
            },
        );
    }

    #[test]
    #[should_panic]
    fn test_validate_market_creation_rejects_wrong_asset_id() {
        let env = Env::default();
        let asset = Address::generate(&env);
        let other = Address::generate(&env);
        let params = sample_market_params(&other, 7);
        let cfg = sample_asset_config(&env);
        validate_market_creation(&env, &asset, &params, &cfg, 7);
    }

    #[test]
    #[should_panic]
    fn test_validate_market_creation_rejects_decimals_out_of_range() {
        let env = Env::default();
        let asset = Address::generate(&env);
        let params = sample_market_params(&asset, MAX_ASSET_DECIMALS + 1);
        let cfg = sample_asset_config(&env);
        validate_market_creation(&env, &asset, &params, &cfg, MAX_ASSET_DECIMALS + 1);
    }
}
