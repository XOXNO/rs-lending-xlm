//! E-mode risk-parameter overrides for correlated asset categories.
//!
//! Storage lives in `storage/emode`; this module applies active category
//! overrides and rejects e-mode combinations that would violate isolation.

use common::errors::{CollateralError, EModeError};
use common::types::{Account, AssetConfig, EModeAssetConfig, EModeCategory};
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env};

use crate::cache::ControllerCache;
use crate::storage;

/// Applies active e-mode collateral and borrow flags to an asset config.
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
        asset_config.loan_to_value = cat.loan_to_value;
        asset_config.liquidation_threshold = cat.liquidation_threshold;
        asset_config.liquidation_bonus = cat.liquidation_bonus;
    }
}

/// Returns market asset config after applicable e-mode overrides.
pub fn effective_asset_config(
    env: &Env,
    account: &Account,
    asset: &Address,
    cache: &mut ControllerCache,
    category: &Option<EModeCategory>,
) -> AssetConfig {
    let mut asset_config = cache.cached_asset_config(asset);
    let asset_emode_config = cache.cached_emode_asset(account.e_mode_category_id, asset);
    apply_e_mode_to_asset_config(env, &mut asset_config, category, asset_emode_config);
    asset_config
}

/// Rejects isolated collateral when an account has an e-mode category.
pub fn ensure_e_mode_compatible_with_asset(env: &Env, asset_config: &AssetConfig, e_mode_category_id: u32) {
    if asset_config.is_isolated_asset && e_mode_category_id > 0 {
        panic_with_error!(env, EModeError::EModeWithIsolated);
    }
}

/// Returns e-mode membership for `asset` after checking the market reverse index.
pub fn token_e_mode_config(
    env: &Env,
    cache: &mut ControllerCache,
    e_mode_category_id: u32,
    asset: &Address,
) -> Option<EModeAssetConfig> {
    if e_mode_category_id == 0 {
        return None;
    }

    // Reverse-index check: asset MarketConfig must record enrollment.
    let market = match cache.market_configs.get(asset.clone()) {
        Some(m) => m,
        None => {
            // Cache misses fall back to stable market config storage.
            match crate::storage::try_get_market_config(env, asset) {
                Some(m) => {
                    cache.market_configs.set(asset.clone(), m.clone());
                    m
                }
                None => panic_with_error!(env, EModeError::EModeCategoryNotFound),
            }
        }
    };
    assert_with_error!(
        env,
        market.asset_config.e_mode_categories.contains(e_mode_category_id),
        EModeError::EModeCategoryNotFound
    );

    let config = cache.cached_emode_asset(e_mode_category_id, asset);
    assert_with_error!(env, config.is_some(), EModeError::EModeCategoryNotFound);
    config
}

/// Returns the e-mode category unless `e_mode_category_id` is zero.
pub fn e_mode_category(env: &Env, e_mode_category_id: u32) -> Option<EModeCategory> {
    if e_mode_category_id == 0 {
        return None;
    }
    Some((&storage::get_emode_category(env, e_mode_category_id)).into())
}

/// Returns a non-deprecated e-mode category unless `e_mode_category_id` is zero.
pub fn active_e_mode_category(env: &Env, e_mode_category_id: u32) -> Option<EModeCategory> {
    let category = e_mode_category(env, e_mode_category_id);
    ensure_e_mode_not_deprecated(env, &category);
    category
}

pub fn ensure_e_mode_not_deprecated(env: &Env, category: &Option<EModeCategory>) {
    if let Some(cat) = category {
        assert_with_error!(env, !cat.is_deprecated, EModeError::EModeCategoryDeprecated);
    }
}

/// Checks that the asset is allowed for supply or borrow in the account's e-mode.
pub fn validate_e_mode_asset(
    env: &Env,
    cache: &mut ControllerCache,
    e_mode_category_id: u32,
    asset: &Address,
    is_supply: bool,
) {
    if e_mode_category_id == 0 {
        return;
    }

    let config = token_e_mode_config(env, cache, e_mode_category_id, asset);
    match config {
        None => {}
        Some(cfg) => {
            if is_supply && !cfg.is_collateralizable {
                panic_with_error!(env, CollateralError::NotCollateral);
            }
            if !is_supply && !cfg.is_borrowable {
                panic_with_error!(env, CollateralError::AssetNotBorrowable);
            }
        }
    }
}

/// Enforces the single-collateral-asset invariant for isolated accounts.
pub fn validate_isolated_collateral(
    env: &Env,
    account: &Account,
    asset: &Address,
    asset_config: &AssetConfig,
) {
    if !account.is_isolated && !asset_config.is_isolated_asset {
        return;
    }

    if !account.is_isolated && asset_config.is_isolated_asset {
        panic_with_error!(env, EModeError::MixIsolatedCollateral);
    }

    // The first deposit on an isolated account always passes.
    if account.supply_positions.is_empty() {
        return;
    }

    for existing_asset in account.supply_positions.keys() {
        assert_with_error!(
            env,
            existing_asset == *asset,
            EModeError::MixIsolatedCollateral
        );
    }
}

/// Rejects accounts that try to combine e-mode with isolation.
pub fn validate_e_mode_isolation_exclusion(env: &Env, e_mode_category_id: u32, is_isolated: bool) {
    if e_mode_category_id > 0 && is_isolated {
        panic_with_error!(env, EModeError::EModeWithIsolated);
    }
}
