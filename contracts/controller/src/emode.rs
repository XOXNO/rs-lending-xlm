//! E-mode risk-parameter overrides for correlated asset categories.
//! Applies active category overrides to market asset configs.

use common::errors::EModeError;
use common::math::fp::Bps;
use controller_interface::types::{Account, AssetConfig, EModeAssetConfig, EModeCategory};
use soroban_sdk::{assert_with_error, Address, Env};

use crate::cache::Cache;
use crate::storage;

/// Applies active e-mode flags to per-asset risk parameters.
pub fn apply_e_mode_to_asset_config(
    _env: &Env,
    asset_config: &mut AssetConfig,
    category: &Option<EModeCategory>,
    asset_emode_config: Option<EModeAssetConfig>,
) {
    if let (Some(cat), Some(aec)) = (category, asset_emode_config) {
        if cat.is_deprecated {
            return;
        }
        asset_config.is_collateralizable = aec.is_collateralizable;
        asset_config.is_borrowable = aec.is_borrowable;
        asset_config.loan_to_value = Bps::from(i128::from(aec.loan_to_value_bps));
        asset_config.liquidation_threshold = Bps::from(i128::from(aec.liquidation_threshold_bps));
        asset_config.liquidation_bonus = Bps::from(i128::from(aec.liquidation_bonus_bps));
    }
}

/// Returns market asset config after applicable e-mode overrides.
pub fn effective_asset_config(
    env: &Env,
    account: &Account,
    asset: &Address,
    cache: &mut Cache,
    category: &Option<EModeCategory>,
) -> AssetConfig {
    let mut asset_config = cache.cached_asset_config(asset);
    let asset_emode_config = cache.cached_emode_asset(account.e_mode_category_id, asset);
    apply_e_mode_to_asset_config(env, &mut asset_config, category, asset_emode_config);
    asset_config
}

pub fn ensure_e_mode_not_deprecated(env: &Env, category: &Option<EModeCategory>) {
    if let Some(cat) = category {
        assert_with_error!(env, !cat.is_deprecated, EModeError::EModeCategoryDeprecated);
    }
}

pub fn validate_e_mode_asset(
    env: &Env,
    cache: &mut Cache,
    e_mode_category_id: u32,
    asset: &Address,
) {
    if e_mode_category_id == 0 {
        return;
    }
    let market = match cache.market_configs.get(asset.clone()) {
        Some(m) => m,
        None => {
            let m = storage::get_market_config(env, asset);
            cache.market_configs.set(asset.clone(), m.clone());
            m
        }
    };
    assert_with_error!(
        env,
        market
            .asset_config
            .e_mode_categories
            .contains(e_mode_category_id),
        EModeError::EModeCategoryNotFound
    );
    assert_with_error!(
        env,
        cache
            .cached_emode_asset(e_mode_category_id, asset)
            .is_some(),
        EModeError::EModeCategoryNotFound
    );
}
