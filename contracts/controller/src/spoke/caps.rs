//! Spoke cap checks and per-asset usage accounting.

use common::errors::SpokeError;
use common::math::fp::Ray;
use common::types::{HubAssetKey, MarketIndexRaw, SpokeAssetConfig, SpokeConfig, SpokeUsageRaw};
use common::validation::cap_is_enabled;
use soroban_sdk::{assert_with_error, panic_with_error, Env, Map};

use crate::storage;

/// Transaction-local buffer for touched `SpokeUsage` rows.
pub(crate) struct SpokeUsageContext {
    spoke_id: u32,
    usage: Map<HubAssetKey, SpokeUsageRaw>,
    /// The spoke config, loaded once on first access. Memoized because the Cache
    /// is fresh per transaction and no governance write to `Spoke` happens inside
    /// a user flow.
    spoke: Option<SpokeConfig>,
    /// Per-asset spoke config, keyed by `hub_asset`. Loaded on first touch. A
    /// missing entry is never memoized, so an unlisted asset still reverts on
    /// every touch.
    spoke_assets: Map<HubAssetKey, SpokeAssetConfig>,
}

impl SpokeUsageContext {
    pub fn new(env: &Env, spoke_id: u32) -> Self {
        Self {
            spoke_id,
            usage: Map::new(env),
            spoke: None,
            spoke_assets: Map::new(env),
        }
    }

    pub fn persist(&self, env: &Env) {
        for (hub_asset, usage) in self.usage.iter() {
            storage::set_spoke_usage(env, self.spoke_id, &hub_asset, &usage);
        }
    }

    pub(crate) fn spoke_id(&self) -> u32 {
        self.spoke_id
    }

    pub(crate) fn spoke(&mut self, env: &Env) -> SpokeConfig {
        if let Some(spoke) = &self.spoke {
            return spoke.clone();
        }
        let spoke = storage::get_spoke(env, self.spoke_id);
        self.spoke = Some(spoke.clone());
        spoke
    }

    pub(crate) fn spoke_asset(
        &mut self,
        env: &Env,
        hub_asset: &HubAssetKey,
    ) -> Option<SpokeAssetConfig> {
        if let Some(cfg) = self.spoke_assets.get(hub_asset.clone()) {
            return Some(cfg);
        }
        let loaded = storage::get_spoke_asset(env, self.spoke_id, hub_asset)?;
        self.spoke_assets.set(hub_asset.clone(), loaded.clone());
        Some(loaded)
    }

    /// Buffered usage for `hub_asset`, lazily loaded from storage (default zero).
    pub(crate) fn spoke_usage(&mut self, env: &Env, hub_asset: &HubAssetKey) -> SpokeUsageRaw {
        if let Some(usage) = self.usage.get(hub_asset.clone()) {
            return usage;
        }
        let loaded = storage::get_spoke_usage(env, self.spoke_id, hub_asset).unwrap_or_default();
        self.usage.set(hub_asset.clone(), loaded.clone());
        loaded
    }

    /// Buffered usage only when an entry already exists (buffer or storage).
    /// Withdraw/repay decrement existing usage but must not create new entries.
    fn spoke_usage_if_present(
        &mut self,
        env: &Env,
        hub_asset: &HubAssetKey,
    ) -> Option<SpokeUsageRaw> {
        if let Some(usage) = self.usage.get(hub_asset.clone()) {
            return Some(usage);
        }
        let loaded = storage::get_spoke_usage(env, self.spoke_id, hub_asset)?;
        self.usage.set(hub_asset.clone(), loaded.clone());
        Some(loaded)
    }

    fn set_usage(&mut self, hub_asset: &HubAssetKey, usage: SpokeUsageRaw) {
        self.usage.set(hub_asset.clone(), usage);
    }

    pub fn apply_supply_after_pool(
        &mut self,
        env: &Env,
        hub_asset: &HubAssetKey,
        delta_scaled: Ray,
        market_index: &MarketIndexRaw,
        decimals: u32,
    ) {
        let cfg = match self.spoke_asset(env, hub_asset) {
            Some(c) => c,
            None => return,
        };
        let mut usage = self.spoke_usage(env, hub_asset);
        enforce_spoke_supply_cap(
            env,
            &usage,
            delta_scaled,
            Ray::from(market_index.supply_index),
            cfg.supply_cap,
            decimals,
        );
        usage.supplied_scaled_ray = usage
            .supplied_scaled_ray
            .checked_add(delta_scaled.raw())
            .unwrap_or_else(|| panic_with_error!(env, common::errors::GenericError::MathOverflow));
        self.set_usage(hub_asset, usage);
    }

    pub fn apply_borrow_after_pool(
        &mut self,
        env: &Env,
        hub_asset: &HubAssetKey,
        delta_scaled: Ray,
        market_index: &MarketIndexRaw,
        decimals: u32,
    ) {
        let cfg = match self.spoke_asset(env, hub_asset) {
            Some(c) => c,
            None => return,
        };
        let mut usage = self.spoke_usage(env, hub_asset);
        enforce_spoke_borrow_cap(
            env,
            &usage,
            delta_scaled,
            Ray::from(market_index.borrow_index),
            cfg.borrow_cap,
            decimals,
        );
        usage.borrowed_scaled_ray = usage
            .borrowed_scaled_ray
            .checked_add(delta_scaled.raw())
            .unwrap_or_else(|| panic_with_error!(env, common::errors::GenericError::MathOverflow));
        self.set_usage(hub_asset, usage);
    }

    pub fn apply_withdraw_after_pool(
        &mut self,
        env: &Env,
        hub_asset: &HubAssetKey,
        delta_scaled: Ray,
    ) {
        if delta_scaled == Ray::ZERO {
            return;
        }
        let Some(mut usage) = self.spoke_usage_if_present(env, hub_asset) else {
            return;
        };
        usage.supplied_scaled_ray = usage
            .supplied_scaled_ray
            .checked_sub(delta_scaled.raw())
            .unwrap_or_else(|| panic_with_error!(env, common::errors::GenericError::MathOverflow));
        self.set_usage(hub_asset, usage);
    }

    pub fn apply_repay_after_pool(
        &mut self,
        env: &Env,
        hub_asset: &HubAssetKey,
        delta_scaled: Ray,
    ) {
        if delta_scaled == Ray::ZERO {
            return;
        }
        let Some(mut usage) = self.spoke_usage_if_present(env, hub_asset) else {
            return;
        };
        usage.borrowed_scaled_ray = usage
            .borrowed_scaled_ray
            .checked_sub(delta_scaled.raw())
            .unwrap_or_else(|| panic_with_error!(env, common::errors::GenericError::MathOverflow));
        self.set_usage(hub_asset, usage);
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
    usage: &SpokeUsageRaw,
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
        SpokeError::SpokeSupplyCapReached
    );
}

fn enforce_spoke_borrow_cap(
    env: &Env,
    usage: &SpokeUsageRaw,
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
        SpokeError::SpokeBorrowCapReached
    );
}

pub fn validate_spoke_caps_against_usage(
    env: &Env,
    usage: &SpokeUsageRaw,
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
            SpokeError::SpokeCapBelowUsage
        );
    }
    if cap_is_enabled(borrow_cap) {
        let cap_scaled = max_scaled_for_cap(env, borrow_cap, decimals, borrow_index);
        assert_with_error!(
            env,
            Ray::from(usage.borrowed_scaled_ray) <= cap_scaled,
            SpokeError::SpokeCapBelowUsage
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
            SpokeError::SpokeCapExceedsHub
        );
    }
    if cap_is_enabled(hub_borrow_cap) && cap_is_enabled(spoke_borrow_cap) {
        assert_with_error!(
            env,
            spoke_borrow_cap <= hub_borrow_cap,
            SpokeError::SpokeCapExceedsHub
        );
    }
}
