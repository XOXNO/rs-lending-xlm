use common::constants::RAY;
use common::errors::{GenericError, SpokeError};
use common::math::fp::Ray;
use common::math::fp_core;
use common::types::{HubAssetKey, MarketIndexRaw, SpokeAssetConfig, SpokeUsageRaw};
use soroban_sdk::{assert_with_error, panic_with_error, Env, Map};

use crate::storage;

#[derive(Clone, Copy)]
pub(crate) enum UsageSide {
    Supply,
    Borrow,
}

impl UsageSide {
    fn scaled(self, usage: &SpokeUsageRaw) -> i128 {
        match self {
            Self::Supply => usage.supplied_scaled_ray,
            Self::Borrow => usage.borrowed_scaled_ray,
        }
    }

    fn set_scaled(self, usage: &mut SpokeUsageRaw, value: i128) {
        match self {
            Self::Supply => usage.supplied_scaled_ray = value,
            Self::Borrow => usage.borrowed_scaled_ray = value,
        }
    }

    pub(crate) fn cap(self, cfg: &SpokeAssetConfig) -> i128 {
        match self {
            Self::Supply => cfg.supply_cap,
            Self::Borrow => cfg.borrow_cap,
        }
    }

    pub(crate) fn index(self, market_index: &MarketIndexRaw) -> Ray {
        match self {
            Self::Supply => Ray::from(market_index.supply_index),
            Self::Borrow => Ray::from(market_index.borrow_index),
        }
    }

    fn cap_error(self) -> SpokeError {
        match self {
            Self::Supply => SpokeError::SpokeSupplyCapReached,
            Self::Borrow => SpokeError::SpokeBorrowCapReached,
        }
    }
}

pub(crate) struct SpokeUsageContext {
    spoke_id: u32,
    usage: Map<HubAssetKey, SpokeUsageRaw>,
}

impl SpokeUsageContext {
    pub(crate) fn new(env: &Env, spoke_id: u32) -> Self {
        Self {
            spoke_id,
            usage: Map::new(env),
        }
    }

    pub(crate) fn persist(&self, env: &Env) {
        for (hub_asset, usage) in self.usage.iter() {
            storage::set_spoke_usage(env, self.spoke_id, &hub_asset, &usage);
        }
    }

    pub(crate) fn spoke_id(&self) -> u32 {
        self.spoke_id
    }

    fn usage_row(&mut self, env: &Env, hub_asset: &HubAssetKey) -> SpokeUsageRaw {
        if let Some(usage) = self.usage.get(hub_asset.clone()) {
            return usage;
        }
        let loaded = storage::get_spoke_usage(env, self.spoke_id, hub_asset).unwrap_or_default();
        self.usage.set(hub_asset.clone(), loaded.clone());
        loaded
    }

    fn usage_row_if_present(
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

    pub(crate) fn apply_entry(
        &mut self,
        env: &Env,
        side: UsageSide,
        hub_asset: &HubAssetKey,
        delta_scaled: Ray,
        cap: i128,
        index: Ray,
        decimals: u32,
    ) {
        let mut usage = self.usage_row(env, hub_asset);
        enforce_spoke_cap(env, side, &usage, delta_scaled, cap, index, decimals);
        let next = side
            .scaled(&usage)
            .checked_add(delta_scaled.raw())
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
        side.set_scaled(&mut usage, next);
        self.set_usage(hub_asset, usage);
    }

    pub(crate) fn apply_exit(
        &mut self,
        env: &Env,
        side: UsageSide,
        hub_asset: &HubAssetKey,
        delta_scaled: Ray,
    ) {
        if delta_scaled == Ray::ZERO {
            return;
        }
        let Some(mut usage) = self.usage_row_if_present(env, hub_asset) else {
            return;
        };
        let next = side
            .scaled(&usage)
            .checked_sub(delta_scaled.raw())
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));

        assert_with_error!(env, next >= 0, GenericError::InternalError);
        side.set_scaled(&mut usage, next);
        self.set_usage(hub_asset, usage);
    }
}

fn cap_to_scaled(env: &Env, cap: i128, decimals: u32, index: Ray) -> Ray {
    Ray::from(fp_core::mul_div_floor_saturating(
        env,
        Ray::from_asset(cap, decimals).raw(),
        RAY,
        index.raw(),
    ))
}

fn enforce_spoke_cap(
    env: &Env,
    side: UsageSide,
    usage: &SpokeUsageRaw,
    delta_scaled: Ray,
    cap: i128,
    index: Ray,
    decimals: u32,
) {
    let cap_scaled = cap_to_scaled(env, cap, decimals, index);
    let next_scaled = Ray::from(side.scaled(usage)).checked_add(env, delta_scaled);
    assert_with_error!(env, next_scaled <= cap_scaled, side.cap_error());
}
