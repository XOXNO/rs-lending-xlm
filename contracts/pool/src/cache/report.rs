//! Index setters and controller-facing mutation / snapshot builders.

use common::math::fp::Ray;
use common::types::{
    MarketIndexRaw, MarketStateSnapshot, PoolPositionMutation, PoolStrategyMutation,
    ScaledPositionRaw,
};

use super::Cache;

impl Cache {
    // --- indexes ---

    /// Replaces the supply index.
    pub(crate) fn set_supply_index(&mut self, index: Ray) {
        self.supply_index = index;
    }

    /// Replaces the borrow index.
    pub(crate) fn set_borrow_index(&mut self, index: Ray) {
        self.borrow_index = index;
    }

    // --- reporting ---

    /// Reports both indexes in their raw ABI form.
    pub(crate) fn market_index(&self) -> MarketIndexRaw {
        MarketIndexRaw {
            borrow_index: self.borrow_index.raw(),
            supply_index: self.supply_index.raw(),
        }
    }

    /// Returns the current in-memory state, committed or not. Prefer
    /// [`Cache::commit`], which persists and reports in one step.
    pub(crate) fn snapshot(&self) -> MarketStateSnapshot {
        MarketStateSnapshot {
            hub_asset: self.hub_asset.clone(),
            timestamp: self.current_timestamp,
            supply_index: self.supply_index.raw(),
            borrow_index: self.borrow_index.raw(),
            // Asset-native cash, not a scaled RAY share.
            cash: self.cash,
            supplied: self.supplied.raw(),
            borrowed: self.borrowed.raw(),
            revenue: self.revenue.raw(),
        }
    }

    /// Builds the controller-facing result of a supply, borrow, withdraw, or
    /// repay leg. `actual_amount` is caller-defined: gross for withdraw and
    /// borrow, net of the refund for repay.
    pub(crate) fn position_mutation(
        &self,
        scaled: Ray,
        actual_amount: i128,
    ) -> PoolPositionMutation {
        PoolPositionMutation {
            position: ScaledPositionRaw {
                scaled_amount: scaled.raw(),
            },
            market_index: self.market_index(),
            actual_amount,
            asset_decimals: self.params.asset_decimals,
        }
    }

    /// Builds the controller-facing result of a strategy borrow leg.
    pub(crate) fn strategy_mutation(
        &self,
        scaled: Ray,
        actual_amount: i128,
        amount_received: i128,
    ) -> PoolStrategyMutation {
        PoolStrategyMutation {
            position: ScaledPositionRaw {
                scaled_amount: scaled.raw(),
            },
            market_index: self.market_index(),
            actual_amount,
            amount_received,
            asset_decimals: self.params.asset_decimals,
        }
    }
}
