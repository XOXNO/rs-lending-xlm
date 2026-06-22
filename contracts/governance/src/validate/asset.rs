//! Risk-bound, asset-config, position-limit, and market-creation validation,
//! plus the live token-shape probe.

use common::constants::{BPS, MAX_FLASHLOAN_FEE_BPS, POSITION_LIMIT_MAX};
use common::errors::{CollateralError, EModeError, FlashLoanError, GenericError};
use common::types::MarketParamsRaw;
use common::validation::cap_is_enabled;
use controller_interface::types::{AssetConfigRaw, PositionLimits};
use controller_interface::{ControllerAdminClient, ControllerClient};
use soroban_sdk::{assert_with_error, panic_with_error, token, Address, Env};

// SAC decimal range for RAY/WAD conversions. Assets below 6 decimals can
// truncate small collateral toward zero in fixed-point valuation.
const MIN_ASSET_DECIMALS: u32 = 6;
const MAX_ASSET_DECIMALS: u32 = 18;

pub(crate) fn validate_risk_bounds(env: &Env, ltv: u32, threshold: u32, bonus: u32) {
    // Uses the shared controller risk-bound validator.
    common::validation::validate_risk_bounds(env, ltv, threshold, bonus);
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

    assert_with_error!(
        env,
        i128::from(config.flashloan_fee_bps) <= MAX_FLASHLOAN_FEE_BPS,
        FlashLoanError::StrategyFeeExceeds
    );
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
    params.verify(env);
}

pub(crate) fn validate_hub_caps(env: &Env, supply_cap: i128, borrow_cap: i128) {
    assert_with_error!(
        env,
        supply_cap >= 0 && borrow_cap >= 0,
        CollateralError::InvalidBorrowParams
    );
}

pub(crate) fn validate_proposed_hub_caps_against_spokes(
    env: &Env,
    controller: &Address,
    asset: &Address,
    hub_supply_cap: i128,
    hub_borrow_cap: i128,
) {
    let market = ControllerAdminClient::new(env, controller).get_market_config(asset);
    let views = ControllerClient::new(env, controller);
    for category_id in market.asset_config.e_mode_categories.iter() {
        let category = views.get_e_mode_category(&category_id);
        let Some(cfg) = category.assets.get(asset.clone()) else {
            continue;
        };
        if cap_is_enabled(hub_supply_cap) && cap_is_enabled(cfg.supply_cap) {
            assert_with_error!(
                env,
                cfg.supply_cap <= hub_supply_cap,
                EModeError::SpokeCapExceedsHub
            );
        }
        if cap_is_enabled(hub_borrow_cap) && cap_is_enabled(cfg.borrow_cap) {
            assert_with_error!(
                env,
                cfg.borrow_cap <= hub_borrow_cap,
                EModeError::SpokeCapExceedsHub
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::constants::RAY;
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
            is_flashloanable: true,
            flashloan_fee_bps: 9,
            asset_decimals: 7,
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
            supply_cap: 5_000_000,
            borrow_cap: 1_000_000,
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
    fn test_validate_hub_caps_rejects_negative_supply_cap() {
        let env = Env::default();
        validate_hub_caps(&env, -1, 0);
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
