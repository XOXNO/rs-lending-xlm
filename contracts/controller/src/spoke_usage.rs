#[cfg(test)]
#[path = "../tests/spoke.rs"]
mod tests;

use common::errors::{GenericError, SpokeError};
use common::math::fp::Ray;
use common::rates::calculate_scaled_cap;
use common::types::{HubAssetKey, MarketIndexRaw, SpokeAssetConfig, SpokeUsageRaw};
use soroban_sdk::{assert_with_error, panic_with_error, Env, Map};

use crate::storage;

#[derive(Clone, Copy)]
pub(crate) enum UsageSide {
    Supply,
    Borrow,
}

impl UsageSide {
    /// Returns this side's scaled amount (RAY) from `usage`.
    fn scaled(self, usage: &SpokeUsageRaw) -> i128 {
        match self {
            Self::Supply => usage.supplied_scaled_ray,
            Self::Borrow => usage.borrowed_scaled_ray,
        }
    }

    /// Sets this side's scaled amount (RAY) on `usage` to `value`.
    fn set_scaled(self, usage: &mut SpokeUsageRaw, value: i128) {
        match self {
            Self::Supply => usage.supplied_scaled_ray = value,
            Self::Borrow => usage.borrowed_scaled_ray = value,
        }
    }

    /// Returns this side's configured cap, in asset units, from `cfg`.
    pub(crate) fn cap(self, cfg: &SpokeAssetConfig) -> i128 {
        match self {
            Self::Supply => cfg.supply_cap,
            Self::Borrow => cfg.borrow_cap,
        }
    }

    /// Returns this side's index (supply or borrow) from `market_index`.
    pub(crate) fn index(self, market_index: &MarketIndexRaw) -> Ray {
        match self {
            Self::Supply => Ray::from(market_index.supply_index),
            Self::Borrow => Ray::from(market_index.borrow_index),
        }
    }

    /// Returns the error to raise when this side's cap is exceeded.
    fn cap_error(self) -> SpokeError {
        match self {
            Self::Supply => SpokeError::SpokeSupplyCapReached,
            Self::Borrow => SpokeError::SpokeBorrowCapReached,
        }
    }
}

#[derive(Clone, Copy)]
enum MissingUsage {
    InsertDefault,
    Absent,
}

pub(crate) struct SpokeUsageContext {
    env: Env,
    spoke_id: u32,
    usage: Map<HubAssetKey, SpokeUsageRaw>,
}

impl SpokeUsageContext {
    /// Creates an empty in-memory usage context for `spoke_id` with no cached rows.
    pub(crate) fn new(env: &Env, spoke_id: u32) -> Self {
        Self {
            env: env.clone(),
            spoke_id,
            usage: Map::new(env),
        }
    }

    /// Writes every cached usage row in this context back to storage.
    pub(crate) fn persist(&self) {
        for (hub_asset, usage) in self.usage.iter() {
            storage::set_spoke_usage(&self.env, self.spoke_id, &hub_asset, &usage);
        }
    }

    /// Returns the spoke id this context tracks usage for.
    pub(crate) fn spoke_id(&self) -> u32 {
        self.spoke_id
    }

    /// Returns the cached usage row for `hub_asset`, loading it from storage into the
    /// cache on first access. If storage has no row, either caches and returns a
    /// zero-valued default row or returns `None`, depending on `missing`.
    fn load_usage_row(
        &mut self,
        hub_asset: &HubAssetKey,
        missing: MissingUsage,
    ) -> Option<SpokeUsageRaw> {
        if let Some(usage) = self.usage.get(hub_asset.clone()) {
            return Some(usage);
        }
        match storage::get_spoke_usage(&self.env, self.spoke_id, hub_asset) {
            Some(loaded) => {
                self.usage.set(hub_asset.clone(), loaded.clone());
                Some(loaded)
            }
            None => match missing {
                MissingUsage::InsertDefault => {
                    let loaded = SpokeUsageRaw::default();
                    self.usage.set(hub_asset.clone(), loaded.clone());
                    Some(loaded)
                }
                MissingUsage::Absent => None,
            },
        }
    }

    /// Increases `side`'s scaled usage for `hub_asset` by `delta_scaled`, panicking with
    /// this side's cap error if the result would exceed `cap`.
    pub(crate) fn apply_entry(
        &mut self,
        side: UsageSide,
        hub_asset: &HubAssetKey,
        delta_scaled: Ray,
        cap: i128,
        index: Ray,
        decimals: u32,
    ) {
        // InsertDefault always yields Some; unwrap_or_default preserves the
        // zero-row entry semantics if that invariant is ever broken.
        let mut usage = self
            .load_usage_row(hub_asset, MissingUsage::InsertDefault)
            .unwrap_or_default();
        let next = enforce_spoke_cap(&self.env, side, &usage, delta_scaled, cap, index, decimals);
        side.set_scaled(&mut usage, next.raw());
        self.usage.set(hub_asset.clone(), usage);
    }

    /// Decreases `side`'s scaled usage for `hub_asset` by `delta_scaled`. No-op if
    /// `delta_scaled` is zero or no usage row exists; panics on overflow or if the
    /// result would go negative.
    pub(crate) fn apply_exit(
        &mut self,
        side: UsageSide,
        hub_asset: &HubAssetKey,
        delta_scaled: Ray,
    ) {
        if delta_scaled == Ray::ZERO {
            return;
        }
        let Some(mut usage) = self.load_usage_row(hub_asset, MissingUsage::Absent) else {
            return;
        };
        let next = side
            .scaled(&usage)
            .checked_sub(delta_scaled.raw())
            .unwrap_or_else(|| panic_with_error!(&self.env, GenericError::MathOverflow));

        assert_with_error!(&self.env, next >= 0, GenericError::InternalError);
        side.set_scaled(&mut usage, next);
        self.usage.set(hub_asset.clone(), usage);
    }
}

/// Returns `side`'s current scaled usage plus `delta_scaled`, panicking with this side's
/// cap error if the result exceeds `cap` (converted to a scaled RAY value via `index` and
/// `decimals`).
fn enforce_spoke_cap(
    env: &Env,
    side: UsageSide,
    usage: &SpokeUsageRaw,
    delta_scaled: Ray,
    cap: i128,
    index: Ray,
    decimals: u32,
) -> Ray {
    let cap_scaled = calculate_scaled_cap(env, cap, decimals, index);
    let next_scaled = Ray::from(side.scaled(usage)).checked_add(env, delta_scaled);
    assert_with_error!(env, next_scaled <= cap_scaled, side.cap_error());
    next_scaled
}
