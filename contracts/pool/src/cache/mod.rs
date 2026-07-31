mod cash;
mod report;
mod scale;
mod shares;

use common::math::fp::Ray;
use common::types::{HubAssetKey, MarketParams, MarketStateSnapshot, PoolState, PoolStateRaw};

use soroban_sdk::Env;

use crate::{storage, time};

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

    pub(crate) fn env(&self) -> &Env {
        &self.env
    }

    pub(crate) fn hub_asset(&self) -> &HubAssetKey {
        &self.hub_asset
    }

    pub(crate) fn params(&self) -> &MarketParams {
        &self.params
    }

    pub(crate) fn supplied(&self) -> Ray {
        self.supplied
    }

    pub(crate) fn borrowed(&self) -> Ray {
        self.borrowed
    }

    pub(crate) fn revenue(&self) -> Ray {
        self.revenue
    }

    pub(crate) fn supply_index(&self) -> Ray {
        self.supply_index
    }

    pub(crate) fn borrow_index(&self) -> Ray {
        self.borrow_index
    }

    pub(crate) fn cash(&self) -> i128 {
        self.cash
    }

    pub(crate) fn elapsed_ms(&self) -> u64 {
        self.current_timestamp.saturating_sub(self.last_timestamp)
    }

    pub(crate) fn needs_accrual(&self) -> bool {
        self.elapsed_ms() > 0
    }

    pub(crate) fn mark_accrued(&mut self) {
        self.last_timestamp = self.current_timestamp;
    }
}

#[cfg(any(test, feature = "certora"))]
impl Cache {
    pub(crate) fn last_timestamp(&self) -> u64 {
        self.last_timestamp
    }
}

#[cfg(test)]
impl Cache {
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

    pub(crate) fn current_timestamp(&self) -> u64 {
        self.current_timestamp
    }

    pub(crate) fn set_current_timestamp(&mut self, timestamp: u64) {
        self.current_timestamp = timestamp;
    }

    pub(crate) fn set_cash(&mut self, cash: i128) {
        self.cash = cash;
    }

    pub(crate) fn set_revenue(&mut self, revenue: Ray) {
        self.revenue = revenue;
    }
}

#[cfg(test)]
#[path = "../../tests/cache.rs"]
mod tests;
