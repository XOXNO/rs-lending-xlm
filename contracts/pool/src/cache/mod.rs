//! In-memory view of one market: load once, mutate through named transitions,
//! commit once. Every field is private so the only way to move money is a
//! method whose name says what moved. Rounding always favours the pool over the
//! counterparty — floor what we pay out, ceil what we are owed.
//!
//! | File | Transitions |
//! |---|---|
//! | [`shares`] | supply / debt / revenue mint-burn |
//! | [`cash`] | reserves, credit/debit, transfer out |
//! | [`scale`] | utilization, scale/unscale, resolve |
//! | [`report`] | snapshot, mutations, index setters |

mod cash;
mod report;
mod scale;
mod shares;

use common::math::fp::Ray;
use common::types::{
    HubAssetKey, MarketParams, MarketStateSnapshot, PoolState, PoolStateRaw,
};

use soroban_sdk::Env;

use crate::{storage, time};

/// One market's params and accounting state, held for the length of a single
/// leg. Construct with [`Cache::load`], mutate with the transitions below,
/// persist with [`Cache::commit`].
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
    /// Tracked cash (`Token(asset)`); direct donations never increase this.
    ///
    /// Invariant: `cash >= sum(claimable supplier + revenue value)`. The surplus
    /// is protocol-owned dead reserve, unreachable by any user path because every
    /// payout is cash-gated by [`Cache::require_reserves`]. Residue from a
    /// floor-clamped bad-debt write-down is rejected on the next supply by
    /// [`crate::guards::require_backed_market`].
    cash: i128,
}

impl Cache {
    /// Loads params and state for `hub_asset` and renews both market keys.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — params or state missing for the market.
    /// * `MathOverflow` — ledger timestamp to milliseconds overflow.
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

    /// Persists the mutated state and returns the snapshot describing it, so a
    /// snapshot can never be published for a transition that was not written.
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

    // --- reads ---

    /// Borrows the environment this cache was loaded with.
    pub(crate) fn env(&self) -> &Env {
        &self.env
    }

    /// Identifies the market this cache holds.
    pub(crate) fn hub_asset(&self) -> &HubAssetKey {
        &self.hub_asset
    }

    /// Borrows the market's risk and rate-model parameters.
    pub(crate) fn params(&self) -> &MarketParams {
        &self.params
    }

    /// Total supply shares outstanding — scaled, not an asset amount.
    pub(crate) fn supplied(&self) -> Ray {
        self.supplied
    }

    /// Total debt shares outstanding — scaled, not an asset amount.
    pub(crate) fn borrowed(&self) -> Ray {
        self.borrowed
    }

    /// The protocol-owned slice of the outstanding supply shares.
    pub(crate) fn revenue(&self) -> Ray {
        self.revenue
    }

    /// Current supply index.
    pub(crate) fn supply_index(&self) -> Ray {
        self.supply_index
    }

    /// Current borrow index.
    pub(crate) fn borrow_index(&self) -> Ray {
        self.borrow_index
    }

    /// Tracked cash in asset decimals.
    pub(crate) fn cash(&self) -> i128 {
        self.cash
    }

    // --- clock ---

    /// Returns milliseconds the market has not yet accrued for.
    pub(crate) fn elapsed_ms(&self) -> u64 {
        self.current_timestamp.saturating_sub(self.last_timestamp)
    }

    /// True while interest is owed for elapsed time.
    pub(crate) fn needs_accrual(&self) -> bool {
        self.elapsed_ms() > 0
    }

    /// Marks the market accrued up to the current ledger time.
    pub(crate) fn mark_accrued(&mut self) {
        self.last_timestamp = self.current_timestamp;
    }
}

/// Raw clock read. Production code works in deltas through
/// [`Cache::elapsed_ms`]; formal specs pin the absolute checkpoint.
#[cfg(any(test, feature = "certora"))]
impl Cache {
    /// Absolute accrual checkpoint in milliseconds.
    pub(crate) fn last_timestamp(&self) -> u64 {
        self.last_timestamp
    }
}

#[cfg(test)]
impl Cache {
    /// Builds a cache from explicit parts, bypassing storage, so unit tests can
    /// exercise one transition in isolation.
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

    /// Ledger time this cache was loaded at, in milliseconds.
    pub(crate) fn current_timestamp(&self) -> u64 {
        self.current_timestamp
    }

    /// Moves the cache clock so a test can drive an elapsed interval.
    pub(crate) fn set_current_timestamp(&mut self, timestamp: u64) {
        self.current_timestamp = timestamp;
    }

    /// Forces tracked cash for a test fixture.
    pub(crate) fn set_cash(&mut self, cash: i128) {
        self.cash = cash;
    }

    /// Forces protocol revenue shares for a test fixture.
    pub(crate) fn set_revenue(&mut self, revenue: Ray) {
        self.revenue = revenue;
    }
}

#[cfg(test)]
#[path = "../../tests/cache.rs"]
mod tests;
