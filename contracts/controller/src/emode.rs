//! Spoke risk-parameter overrides for correlated asset spokes.
//! Applies active spoke overrides to market asset configs.

use common::errors::EModeError;
use common::math::fp::Bps;
use controller_interface::types::{Account, AssetConfig, HubAssetKey, SpokeAssetConfig, SpokeConfig};
use soroban_sdk::{assert_with_error, Address, Env};

use crate::cache::Cache;
use crate::storage;

/// Applies active spoke flags to per-asset risk parameters.
pub fn apply_spoke_to_asset_config(
    _env: &Env,
    asset_config: &mut AssetConfig,
    spoke: &Option<SpokeConfig>,
    asset_spoke_config: Option<SpokeAssetConfig>,
) {
    if let (Some(spoke), Some(aec)) = (spoke, asset_spoke_config) {
        if spoke.is_deprecated {
            return;
        }
        asset_config.is_collateralizable = aec.is_collateralizable;
        asset_config.is_borrowable = aec.is_borrowable;
        asset_config.loan_to_value = Bps::from(i128::from(aec.loan_to_value_bps));
        asset_config.liquidation_threshold = Bps::from(i128::from(aec.liquidation_threshold_bps));
        asset_config.liquidation_bonus = Bps::from(i128::from(aec.liquidation_bonus_bps));
    }
}

/// Returns market asset config after applicable spoke overrides.
pub fn effective_asset_config(
    env: &Env,
    account: &Account,
    asset: &Address,
    cache: &mut Cache,
    spoke: &Option<SpokeConfig>,
) -> AssetConfig {
    let mut asset_config = cache.cached_asset_config(asset);
    let hub_asset = HubAssetKey {
        hub_id: 0,
        asset: asset.clone(),
    };
    let asset_spoke_config = cache.cached_spoke_asset(account.spoke_id, &hub_asset);
    apply_spoke_to_asset_config(env, &mut asset_config, spoke, asset_spoke_config);
    asset_config
}

pub fn ensure_spoke_not_deprecated(env: &Env, spoke: &Option<SpokeConfig>) {
    if let Some(spoke) = spoke {
        assert_with_error!(
            env,
            !spoke.is_deprecated,
            EModeError::EModeCategoryDeprecated
        );
    }
}

pub fn validate_spoke_asset(env: &Env, cache: &mut Cache, spoke_id: u32, asset: &Address) {
    if spoke_id == 0 {
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
        market.asset_config.e_mode_categories.contains(spoke_id),
        EModeError::EModeCategoryNotFound
    );
    let hub_asset = HubAssetKey {
        hub_id: 0,
        asset: asset.clone(),
    };
    assert_with_error!(
        env,
        cache.cached_spoke_asset(spoke_id, &hub_asset).is_some(),
        EModeError::EModeCategoryNotFound
    );
}
