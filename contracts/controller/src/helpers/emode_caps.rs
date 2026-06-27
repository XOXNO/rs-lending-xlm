//! E-mode spoke cap enforcement and usage accounting.
//!
//! Hub caps are enforced inside the pool. Spoke caps are checked in scaled
//! space after the pool returns post-accrual indexes.

use common::errors::EModeError;
use common::math::fp::Ray;
use common::validation::cap_is_enabled;
use controller_interface::types::{
    EModeAssetConfig, EModeCategory, EModeCategoryRaw, EModeSpokeUsageRaw, MarketIndexRaw,
};
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env};

use crate::storage;

/// Tracks in-memory spoke usage for one transaction.
pub(crate) struct EModeUsageContext {
    category_id: u32,
    category: EModeCategoryRaw,
}

impl EModeUsageContext {
    pub fn load(env: &Env, category_id: u32) -> Option<Self> {
        if category_id == 0 {
            return None;
        }
        Some(Self {
            category_id,
            category: storage::get_emode_category(env, category_id),
        })
    }

    pub fn persist(&self, env: &Env) {
        storage::set_emode_category(env, self.category_id, &self.category);
    }

    pub(crate) fn category_id(&self) -> u32 {
        self.category_id
    }

    pub(crate) fn as_category(&self) -> EModeCategory {
        (&self.category).into()
    }

    pub(crate) fn emode_asset(&self, asset: &Address) -> Option<EModeAssetConfig> {
        self.category.assets.get(asset.clone())
    }

    pub(crate) fn spoke_usage(&self, asset: &Address) -> EModeSpokeUsageRaw {
        self.category
            .usage
            .get(asset.clone())
            .unwrap_or(EModeSpokeUsageRaw {
                supplied_scaled_ray: 0,
                borrowed_scaled_ray: 0,
            })
    }

    fn set_usage(&mut self, asset: &Address, usage: EModeSpokeUsageRaw) {
        if usage.supplied_scaled_ray == 0 && usage.borrowed_scaled_ray == 0 {
            self.category.usage.remove(asset.clone());
        } else {
            self.category.usage.set(asset.clone(), usage);
        }
    }

    fn has_usage_entry(&self, asset: &Address) -> bool {
        self.category.usage.contains_key(asset.clone())
    }

    pub fn apply_supply_after_pool(
        &mut self,
        env: &Env,
        asset: &Address,
        delta_scaled: Ray,
        market_index: &MarketIndexRaw,
        decimals: u32,
    ) {
        let cfg = match self.emode_asset(asset) {
            Some(c) => c,
            None => return,
        };
        let mut usage = self.spoke_usage(asset);
        enforce_spoke_supply_cap(
            env,
            &usage,
            delta_scaled,
            Ray::from(market_index.supply_index_ray),
            cfg.supply_cap,
            decimals,
        );
        usage.supplied_scaled_ray = usage
            .supplied_scaled_ray
            .checked_add(delta_scaled.raw())
            .unwrap_or_else(|| panic_with_error!(env, common::errors::GenericError::MathOverflow));
        self.set_usage(asset, usage);
    }

    pub fn apply_borrow_after_pool(
        &mut self,
        env: &Env,
        asset: &Address,
        delta_scaled: Ray,
        market_index: &MarketIndexRaw,
        decimals: u32,
    ) {
        let cfg = match self.emode_asset(asset) {
            Some(c) => c,
            None => return,
        };
        let mut usage = self.spoke_usage(asset);
        enforce_spoke_borrow_cap(
            env,
            &usage,
            delta_scaled,
            Ray::from(market_index.borrow_index_ray),
            cfg.borrow_cap,
            decimals,
        );
        usage.borrowed_scaled_ray = usage
            .borrowed_scaled_ray
            .checked_add(delta_scaled.raw())
            .unwrap_or_else(|| panic_with_error!(env, common::errors::GenericError::MathOverflow));
        self.set_usage(asset, usage);
    }

    pub fn apply_withdraw_after_pool(&mut self, env: &Env, asset: &Address, delta_scaled: Ray) {
        if delta_scaled == Ray::ZERO || !self.has_usage_entry(asset) {
            return;
        }
        let mut usage = self.spoke_usage(asset);
        usage.supplied_scaled_ray = usage
            .supplied_scaled_ray
            .checked_sub(delta_scaled.raw())
            .unwrap_or_else(|| panic_with_error!(env, common::errors::GenericError::MathOverflow));
        self.set_usage(asset, usage);
    }

    pub fn apply_repay_after_pool(&mut self, env: &Env, asset: &Address, delta_scaled: Ray) {
        if delta_scaled == Ray::ZERO || !self.has_usage_entry(asset) {
            return;
        }
        let mut usage = self.spoke_usage(asset);
        usage.borrowed_scaled_ray = usage
            .borrowed_scaled_ray
            .checked_sub(delta_scaled.raw())
            .unwrap_or_else(|| panic_with_error!(env, common::errors::GenericError::MathOverflow));
        self.set_usage(asset, usage);
    }
}

fn max_scaled_for_cap(env: &Env, cap: i128, decimals: u32, index: Ray) -> Ray {
    if !cap_is_enabled(cap) {
        return Ray::from(i128::MAX);
    }
    // dimensional: Token(asset) cap -> Ray<Token(asset)> -> Ray<Share(asset, side)>.
    Ray::from_asset(cap, decimals).div_floor(env, index)
}

fn enforce_spoke_supply_cap(
    env: &Env,
    usage: &EModeSpokeUsageRaw,
    delta_scaled: Ray,
    supply_index: Ray,
    cap: i128,
    decimals: u32,
) {
    if !cap_is_enabled(cap) {
        return;
    }
    let cap_scaled = max_scaled_for_cap(env, cap, decimals, supply_index);
    let next_scaled = Ray::from(usage.supplied_scaled_ray) + delta_scaled;
    assert_with_error!(
        env,
        next_scaled <= cap_scaled,
        EModeError::SpokeSupplyCapReached
    );
}

fn enforce_spoke_borrow_cap(
    env: &Env,
    usage: &EModeSpokeUsageRaw,
    delta_scaled: Ray,
    borrow_index: Ray,
    cap: i128,
    decimals: u32,
) {
    if !cap_is_enabled(cap) {
        return;
    }
    let cap_scaled = max_scaled_for_cap(env, cap, decimals, borrow_index);
    let next_scaled = Ray::from(usage.borrowed_scaled_ray) + delta_scaled;
    assert_with_error!(
        env,
        next_scaled <= cap_scaled,
        EModeError::SpokeBorrowCapReached
    );
}

/// Rejects hub caps that would sit below an asset's configured spoke caps.
pub fn validate_hub_caps_against_category_spokes(
    env: &Env,
    asset: &Address,
    hub_supply_cap: i128,
    hub_borrow_cap: i128,
) {
    let market = storage::get_market_config(env, asset);
    for category_id in market.asset_config.e_mode_categories.iter() {
        if let Some(cfg) = storage::get_emode_asset(env, category_id, asset) {
            validate_spoke_caps_against_hub(
                env,
                hub_supply_cap,
                hub_borrow_cap,
                cfg.supply_cap,
                cfg.borrow_cap,
            );
        }
    }
}

pub fn validate_spoke_caps_against_usage(
    env: &Env,
    usage: &EModeSpokeUsageRaw,
    supply_cap: i128,
    borrow_cap: i128,
    supply_index: Ray,
    borrow_index: Ray,
    decimals: u32,
) {
    if cap_is_enabled(supply_cap) {
        let cap_scaled = max_scaled_for_cap(env, supply_cap, decimals, supply_index);
        assert_with_error!(
            env,
            Ray::from(usage.supplied_scaled_ray) <= cap_scaled,
            EModeError::SpokeCapBelowUsage
        );
    }
    if cap_is_enabled(borrow_cap) {
        let cap_scaled = max_scaled_for_cap(env, borrow_cap, decimals, borrow_index);
        assert_with_error!(
            env,
            Ray::from(usage.borrowed_scaled_ray) <= cap_scaled,
            EModeError::SpokeCapBelowUsage
        );
    }
}

pub fn validate_spoke_caps_against_hub(
    env: &Env,
    hub_supply_cap: i128,
    hub_borrow_cap: i128,
    spoke_supply_cap: i128,
    spoke_borrow_cap: i128,
) {
    if cap_is_enabled(hub_supply_cap) && cap_is_enabled(spoke_supply_cap) {
        assert_with_error!(
            env,
            spoke_supply_cap <= hub_supply_cap,
            EModeError::SpokeCapExceedsHub
        );
    }
    if cap_is_enabled(hub_borrow_cap) && cap_is_enabled(spoke_borrow_cap) {
        assert_with_error!(
            env,
            spoke_borrow_cap <= hub_borrow_cap,
            EModeError::SpokeCapExceedsHub
        );
    }
}
