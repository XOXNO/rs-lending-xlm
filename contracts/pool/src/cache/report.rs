use common::math::fp::Ray;
use common::types::{
    MarketIndexRaw, MarketStateSnapshot, PoolPositionMutation, PoolStrategyMutation,
    ScaledPositionRaw,
};

use super::Cache;

impl Cache {
    pub(crate) fn set_supply_index(&mut self, index: Ray) {
        self.supply_index = index;
    }

    pub(crate) fn set_borrow_index(&mut self, index: Ray) {
        self.borrow_index = index;
    }

    pub(crate) fn market_index(&self) -> MarketIndexRaw {
        MarketIndexRaw {
            borrow_index: self.borrow_index.raw(),
            supply_index: self.supply_index.raw(),
        }
    }

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
