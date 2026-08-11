//! Per-spoke, per-asset usage tracking and cap enforcement for supply and borrow sides.
//!
//! `SpokeUsageContext` caches scaled spoke usage rows in memory for the duration of a
//! controller invocation, loading from and deferring writes to storage, while
//! `enforce_spoke_cap` checks scaled usage against a configured asset-unit cap.

use common::errors::{GenericError, SpokeError};
use common::math::fp::Ray;
use common::rates::calculate_scaled_cap;
use common::types::{HubAssetKey, MarketIndexRaw, SpokeAssetConfig, SpokeUsageRaw};
use soroban_sdk::{assert_with_error, panic_with_error, Env, Map};

use crate::storage;

/// Selects which side of a spoke usage row (supplied or borrowed scaled amount) an
/// operation reads or mutates.
#[derive(Clone, Copy)]
pub(crate) enum UsageSide {
    Supply,
    Borrow,
}

impl UsageSide {
    /// Returns the scaled amount for this side from `usage`.
    fn scaled(self, usage: &SpokeUsageRaw) -> i128 {
        match self {
            Self::Supply => usage.supplied_scaled_ray,
            Self::Borrow => usage.borrowed_scaled_ray,
        }
    }

    /// Sets the scaled amount for this side on `usage`.
    fn set_scaled(self, usage: &mut SpokeUsageRaw, value: i128) {
        match self {
            Self::Supply => usage.supplied_scaled_ray = value,
            Self::Borrow => usage.borrowed_scaled_ray = value,
        }
    }

    /// Returns the configured cap for this side from `cfg`.
    pub(crate) fn cap(self, cfg: &SpokeAssetConfig) -> i128 {
        match self {
            Self::Supply => cfg.supply_cap,
            Self::Borrow => cfg.borrow_cap,
        }
    }

    /// Returns the market index for this side, converted to `Ray`.
    pub(crate) fn index(self, market_index: &MarketIndexRaw) -> Ray {
        match self {
            Self::Supply => Ray::from(market_index.supply_index),
            Self::Borrow => Ray::from(market_index.borrow_index),
        }
    }

    /// Returns the cap-breach error variant for this side.
    fn cap_error(self) -> SpokeError {
        match self {
            Self::Supply => SpokeError::SpokeSupplyCapReached,
            Self::Borrow => SpokeError::SpokeBorrowCapReached,
        }
    }
}

/// Policy for a spoke usage row that is neither cached nor stored.
///
/// - `InsertDefault`: entry path. Treat missing storage as zero usage and
///   cache that synthetic row so subsequent updates in the same context see it.
/// - `Absent`: exit path. Leave missing storage missing and do not cache a
///   synthetic zero row (exit becomes a no-op when no usage exists).
#[derive(Clone, Copy)]
enum MissingUsage {
    InsertDefault,
    Absent,
}

/// In-memory cache of scaled spoke usage rows, keyed by hub asset, for a single spoke.
/// Loads rows from storage on first access and defers writes until `persist` is called.
pub(crate) struct SpokeUsageContext {
    spoke_id: u32,
    usage: Map<HubAssetKey, SpokeUsageRaw>,
}

impl SpokeUsageContext {
    /// Creates an empty usage cache for `spoke_id`.
    pub(crate) fn new(env: &Env, spoke_id: u32) -> Self {
        Self {
            spoke_id,
            usage: Map::new(env),
        }
    }

    /// Writes every cached usage row back to storage.
    pub(crate) fn persist(&self, env: &Env) {
        for (hub_asset, usage) in self.usage.iter() {
            storage::set_spoke_usage(env, self.spoke_id, &hub_asset, &usage);
        }
    }

    pub(crate) fn spoke_id(&self) -> u32 {
        self.spoke_id
    }

    /// Shared cache/storage loader for spoke usage rows.
    ///
    /// Always prefers the in-memory map. On a storage miss, `missing`
    /// selects between entry default-insert and exit no-insert.
    fn load_usage_row(
        &mut self,
        env: &Env,
        hub_asset: &HubAssetKey,
        missing: MissingUsage,
    ) -> Option<SpokeUsageRaw> {
        if let Some(usage) = self.usage.get(hub_asset.clone()) {
            return Some(usage);
        }
        match storage::get_spoke_usage(env, self.spoke_id, hub_asset) {
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

    /// Updates the cached usage row for `hub_asset` to `usage`.
    fn set_usage(&mut self, hub_asset: &HubAssetKey, usage: SpokeUsageRaw) {
        self.usage.set(hub_asset.clone(), usage);
    }

    /// Loads (or defaults) the cached usage row for `hub_asset`, adds `delta_scaled` to
    /// the scaled amount for `side` while enforcing `cap`, and writes the updated row
    /// back to the cache. Panics if the resulting scaled usage exceeds `cap` or the
    /// addition overflows.
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
        // InsertDefault always yields Some; unwrap_or_default preserves the
        // zero-row entry semantics if that invariant is ever broken.
        let mut usage = self
            .load_usage_row(env, hub_asset, MissingUsage::InsertDefault)
            .unwrap_or_default();
        let next = enforce_spoke_cap(env, side, &usage, delta_scaled, cap, index, decimals);
        side.set_scaled(&mut usage, next.raw());
        self.set_usage(hub_asset, usage);
    }

    /// Subtracts `delta_scaled` from the scaled amount for `side` on the cached usage
    /// row for `hub_asset`. A no-op if `delta_scaled` is zero or no usage row exists in
    /// storage or the cache. Panics if the subtraction overflows or would go negative.
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
        let Some(mut usage) = self.load_usage_row(env, hub_asset, MissingUsage::Absent) else {
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

/// Returns `usage + delta` after enforcing the asset-unit cap.
/// Overflow → `GenericError::MathOverflow`; breach → side cap error.
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
