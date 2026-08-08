//! In-memory market cache: load → mutate → commit.
//!
//! [`Cache`] is the working set for a single hub-asset market during one
//! operation leg. Submodules extend it:
//!
//! - [`cash`] — reserve checks and token transfers
//! - [`scale`] — share ↔ asset conversions and utilization
//! - [`shares`] — mint/burn supply, debt, and revenue shares
//! - [`report`] — snapshots and position mutation DTOs

mod cash;
mod report;
mod scale;
mod shares;

use common::math::fp::Ray;
use common::types::{HubAssetKey, MarketParams, MarketStateSnapshot, PoolState, PoolStateRaw};

use soroban_sdk::Env;

use crate::{storage, time};

/// Mutable snapshot of one market for the duration of a mutation or view.
///
/// Constructed from persistent storage, updated by ops/interest, then written
/// back via [`Cache::commit`]. Does not hold a storage lock; callers must not
/// interleave commits for the same market without reloading.
pub(crate) struct Cache {
    env: Env,
    hub_asset: HubAssetKey,
    params: MarketParams,
    last_timestamp: u64,
    current_timestamp: u64,
    supplied: Ray,
    borrowed: Ray,
    revenue: Ray,
    borrow_index: Ray,
    supply_index: Ray,
    cash: i128,
}

impl Cache {
    /// Load params and state from storage, renew market TTLs, stamp current time.
    ///
    /// Does **not** accrue interest; call [`crate::interest::global_sync`] or
    /// [`crate::ops::synced_market`] when accrual is required.
    pub(crate) fn load(env: &Env, hub_asset: &HubAssetKey) -> Self {
        let raw_params = storage::read_params(env, hub_asset);
        let raw_state = storage::read_state(env, hub_asset);
        storage::renew_market(env, hub_asset);

        let state = PoolState::from(&raw_state);
        let params = MarketParams::from(&raw_params);
        let time = time::now_ms(env);

        Self {
            env: env.clone(),
            hub_asset: hub_asset.clone(),
            params,
            last_timestamp: state.last_timestamp,
            current_timestamp: time,
            supplied: state.supplied,
            borrowed: state.borrowed,
            revenue: state.revenue,
            borrow_index: state.borrow_index,
            supply_index: state.supply_index,
            cash: state.cash,
        }
    }

    /// Persist the full market state and return a snapshot for events.
    pub(crate) fn commit(&self) -> MarketStateSnapshot {
        let state = PoolStateRaw {
            supplied: self.supplied.raw(),
            borrowed: self.borrowed.raw(),
            revenue: self.revenue.raw(),
            borrow_index: self.borrow_index.raw(),
            supply_index: self.supply_index.raw(),
            last_timestamp: self.last_timestamp,
            cash: self.cash,
        };
        storage::write_state(&self.env, &self.hub_asset, &state);
        self.snapshot()
    }

    /// Host environment used for math and token clients.
    pub(crate) fn env(&self) -> &Env {
        &self.env
    }

    /// Hub + asset identity for this market.
    pub(crate) fn hub_asset(&self) -> &HubAssetKey {
        &self.hub_asset
    }

    /// Interest-rate params and token metadata for this market.
    pub(crate) fn params(&self) -> &MarketParams {
        &self.params
    }

    /// Total scaled supply shares (including protocol revenue shares).
    pub(crate) fn supplied(&self) -> Ray {
        self.supplied
    }

    /// Total scaled debt shares.
    pub(crate) fn borrowed(&self) -> Ray {
        self.borrowed
    }

    /// Scaled protocol revenue shares (subset of supply).
    pub(crate) fn revenue(&self) -> Ray {
        self.revenue
    }

    /// Supply exchange rate index (RAY).
    pub(crate) fn supply_index(&self) -> Ray {
        self.supply_index
    }

    /// Borrow exchange rate index (RAY).
    pub(crate) fn borrow_index(&self) -> Ray {
        self.borrow_index
    }

    /// Cash reserves in asset units (accounting book, not live token balance).
    pub(crate) fn cash(&self) -> i128 {
        self.cash
    }

    /// Milliseconds between last accrual and the stamped current time.
    pub(crate) fn elapsed_ms(&self) -> u64 {
        self.current_timestamp.saturating_sub(self.last_timestamp)
    }

    /// `true` when interest should be compounded before further mutations.
    pub(crate) fn needs_accrual(&self) -> bool {
        self.elapsed_ms() > 0
    }

    /// Mark the market as fully accrued through `current_timestamp`.
    pub(crate) fn mark_accrued(&mut self) {
        self.last_timestamp = self.current_timestamp;
    }
}

#[cfg(any(test, feature = "certora"))]
impl Cache {
    /// Last committed accrual timestamp (ms). Test / formal-verification only.
    pub(crate) fn last_timestamp(&self) -> u64 {
        self.last_timestamp
    }
}

#[cfg(test)]
impl Cache {
    /// Build a cache from explicit parts without reading storage.
    pub(crate) fn from_parts(
        env: &Env,
        hub_asset: HubAssetKey,
        params: &common::types::MarketParamsRaw,
        state: &PoolStateRaw,
        current_timestamp: u64,
    ) -> Self {
        let parts = PoolState::from(state);
        Self {
            env: env.clone(),
            hub_asset,
            params: MarketParams::from(params),
            last_timestamp: parts.last_timestamp,
            current_timestamp,
            supplied: parts.supplied,
            borrowed: parts.borrowed,
            revenue: parts.revenue,
            borrow_index: parts.borrow_index,
            supply_index: parts.supply_index,
            cash: parts.cash,
        }
    }

    /// Stamped current timestamp used for elapsed-time calculations.
    pub(crate) fn current_timestamp(&self) -> u64 {
        self.current_timestamp
    }

    /// Override current timestamp (tests that advance time without ledger ticks).
    pub(crate) fn set_current_timestamp(&mut self, timestamp: u64) {
        self.current_timestamp = timestamp;
    }

    /// Force cash reserves to an exact value.
    pub(crate) fn set_cash(&mut self, cash: i128) {
        self.cash = cash;
    }

    /// Force revenue shares to an exact value.
    pub(crate) fn set_revenue(&mut self, revenue: Ray) {
        self.revenue = revenue;
    }
}

#[cfg(test)]
#[path = "../../tests/cache.rs"]
mod tests;
