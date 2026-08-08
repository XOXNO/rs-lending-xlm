//! Index snapshots and position mutation DTOs built from the cache.
//!
//! These helpers package post-mutation state for hub return values and events
//! without re-reading storage.

use common::math::fp::Ray;
use common::types::{
    MarketIndexRaw, MarketStateSnapshot, PoolPositionMutation, PoolStrategyMutation,
    ScaledPositionRaw,
};

use super::Cache;

impl Cache {
    /// Set the supply exchange-rate index after accrual or bad-debt socialization.
    pub(crate) fn set_supply_index(&mut self, index: Ray) {
        self.supply_index = index;
    }

    /// Set the borrow exchange-rate index after accrual.
    pub(crate) fn set_borrow_index(&mut self, index: Ray) {
        self.borrow_index = index;
    }

    /// Current borrow/supply indexes as a raw DTO for hub sync.
    pub(crate) fn market_index(&self) -> MarketIndexRaw {
        MarketIndexRaw {
            borrow_index: self.borrow_index.raw(),
            supply_index: self.supply_index.raw(),
        }
    }

    /// Full market state snapshot for event emission (does not write storage).
    pub(crate) fn snapshot(&self) -> MarketStateSnapshot {
        MarketStateSnapshot {
            hub_asset: self.hub_asset.clone(),
            timestamp: self.current_timestamp,
            supply_index: self.supply_index.raw(),
            borrow_index: self.borrow_index.raw(),
            cash: self.cash,
            supplied: self.supplied.raw(),
            borrowed: self.borrowed.raw(),
            revenue: self.revenue.raw(),
        }
    }

    /// Build a supply/borrow position mutation for a batch leg result.
    ///
    /// * `scaled` — user's remaining scaled position after the leg
    /// * `actual_amount` — asset units applied (minted, repaid, withdrawn, …)
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

    /// Build a strategy mutation including net amount received after fees.
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
