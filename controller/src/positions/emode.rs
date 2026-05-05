use common::errors::{CollateralError, EModeError};
use common::types::{Account, AssetConfig, EModeAssetConfig, EModeCategory};
use soroban_sdk::{panic_with_error, Address, Env};

use crate::cache::ControllerCache;
use crate::storage;

// ---------------------------------------------------------------------------
// Core e-mode functions
// ---------------------------------------------------------------------------

/// Overrides `asset_config` risk parameters with the e-mode category's boosted LTV and thresholds.
/// No-ops when either `category` or `asset_emode_config` is `None`, or when the category is deprecated.
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
        asset_config.loan_to_value_bps = cat.loan_to_value_bps;
        asset_config.liquidation_threshold_bps = cat.liquidation_threshold_bps;
        asset_config.liquidation_bonus_bps = cat.liquidation_bonus_bps;
    }
}

/// Returns the asset config after applying the account's active e-mode overrides.
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

/// Panics with `EModeWithIsolated` when an isolated asset is assigned to a non-zero e-mode category.
pub fn ensure_e_mode_compatible_with_asset(env: &Env, asset_config: &AssetConfig, e_mode_id: u32) {
    if asset_config.is_isolated_asset && e_mode_id > 0 {
        panic_with_error!(env, EModeError::EModeWithIsolated);
    }
}

/// Returns the e-mode membership config for `asset` in category `e_mode_id`.
/// Panics with `EModeCategoryNotFound` when the asset is not a member of the category.
pub fn token_e_mode_config(env: &Env, e_mode_id: u32, asset: &Address) -> Option<EModeAssetConfig> {
    if e_mode_id == 0 {
        return None;
    }

    // Reverse-index check: the asset's MarketConfig records every
    // category it's enrolled in, so a missing entry rejects the call
    // before the heavier per-category lookup.
    let market = match storage::try_get_market_config(env, asset) {
        Some(m) => m,
        None => panic_with_error!(env, EModeError::EModeCategoryNotFound),
    };
    if !market.asset_config.e_mode_categories.contains(e_mode_id) {
        panic_with_error!(env, EModeError::EModeCategoryNotFound);
    }

    let config = storage::get_emode_asset(env, e_mode_id, asset);
    if config.is_none() {
        panic_with_error!(env, EModeError::EModeCategoryNotFound);
    }
    config
}

/// Returns the `EModeCategory` for `e_mode_id`, or `None` when `e_mode_id` is zero (no e-mode).
pub fn e_mode_category(env: &Env, e_mode_id: u32) -> Option<EModeCategory> {
    if e_mode_id == 0 {
        return None;
    }
    Some(storage::get_emode_category(env, e_mode_id))
}

/// Returns the account's e-mode category and rejects deprecated categories.
pub fn active_e_mode_category(env: &Env, e_mode_id: u32) -> Option<EModeCategory> {
    let category = e_mode_category(env, e_mode_id);
    ensure_e_mode_not_deprecated(env, &category);
    category
}

// ---------------------------------------------------------------------------
// Deprecation check
// ---------------------------------------------------------------------------

/// Panics with `EModeCategoryDeprecated` when `category` is `Some` and marked deprecated.
pub fn ensure_e_mode_not_deprecated(env: &Env, category: &Option<EModeCategory>) {
    if let Some(cat) = category {
        if cat.is_deprecated {
            panic_with_error!(env, EModeError::EModeCategoryDeprecated);
        }
    }
}

// ---------------------------------------------------------------------------
// Convenience helpers (used by strategy.rs and other callers)
// ---------------------------------------------------------------------------

/// Panics with `NotCollateral` or `AssetNotBorrowable` when the asset's e-mode membership
/// disallows the requested operation. `is_supply = true` checks collateralizability; `false` checks borrowability.
pub fn validate_e_mode_asset(env: &Env, e_mode_category_id: u32, asset: &Address, is_supply: bool) {
    if e_mode_category_id == 0 {
        return;
    }

    let config = token_e_mode_config(env, e_mode_category_id, asset);
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

// ---------------------------------------------------------------------------
// Isolation mode enforcement (accepts pre-loaded data from caller)
// ---------------------------------------------------------------------------

/// Panics with `MixIsolatedCollateral` when an isolated account supplies a different
/// asset than its existing collateral, or when a non-isolated account supplies an isolated asset.
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
        if existing_asset != *asset {
            panic_with_error!(env, EModeError::MixIsolatedCollateral);
        }
    }
}

/// Panics with `EModeWithIsolated` when both `e_mode_category > 0` and `is_isolated` are true.
pub fn validate_e_mode_isolation_exclusion(env: &Env, e_mode_category: u32, is_isolated: bool) {
    if e_mode_category > 0 && is_isolated {
        panic_with_error!(env, EModeError::EModeWithIsolated);
    }
}
