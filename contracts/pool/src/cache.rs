//! In-memory market cache: loads params and interest state for one hub-asset,
//! runs scaled-share/reserve accounting and rounding conversions, and persists
//! the result. Rounding direction (half-up, floor, ceil) is chosen per flow to
//! keep protocol solvency conservative.

use common::errors::{CollateralError, GenericError};
use common::math::fp::Ray;
use common::rates::{scaled_to_original, utilization as rate_utilization};
use common::types::{
    HubAssetKey, MarketIndexRaw, MarketParams, MarketParamsRaw, MarketStateSnapshot, PoolKey,
    PoolPositionMutation, PoolState, PoolStateRaw, PoolStrategyMutation, ScaledPositionRaw,
};

use soroban_sdk::{assert_with_error, panic_with_error, token, Address, Env};

use crate::utils;

/// In-memory market params + interest state; one load/save per market leg.
pub struct Cache {
    pub env: Env,
    pub supplied: Ray,
    pub borrowed: Ray,
    pub revenue: Ray,
    pub borrow_index: Ray,
    pub supply_index: Ray,
    pub last_timestamp: u64,
    pub current_timestamp: u64,
    pub params: MarketParams,
    pub hub_asset: HubAssetKey,
    /// Tracked reserves (`Token(asset)`); direct donations never increase this.
    ///
    /// Invariant: `cash >= sum(claimable supplier + revenue value)`. The surplus
    /// is protocol-owned dead reserve that accrues from conservative rounding
    /// (floor payouts) and the `update_supply_index` virtual-offset dilution
    /// (rewards on a near-empty market are under-distributed). It is never
    /// extractable by any user path — every withdrawal is cash-gated by
    /// `require_reserves` — and would be recovered only by a future governance
    /// reserve-skim entrypoint. Keeping it conservative preserves solvency.
    pub cash: i128,
}

impl Cache {
    /// # Errors
    /// * `PoolNotInitialized` - params or state missing for the market.
    pub fn load(env: &Env, hub_asset: &HubAssetKey) -> Self {
        let params: MarketParamsRaw = env
            .storage()
            .persistent()
            .get(&PoolKey::Params(hub_asset.clone()))
            .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized));

        let raw_state: PoolStateRaw = env
            .storage()
            .persistent()
            .get(&PoolKey::State(hub_asset.clone()))
            .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized));
        // Renew after successful loads; `extend_ttl` requires existing keys.
        utils::renew_market_keys(env, hub_asset);
        let state = PoolState::from(&raw_state);
        let market_params = MarketParams::from(&params);
        let timestamp = utils::now_ms(env);

        Self {
            env: env.clone(),
            supplied: state.supplied,
            borrowed: state.borrowed,
            revenue: state.revenue,
            borrow_index: state.borrow_index,
            supply_index: state.supply_index,
            last_timestamp: state.last_timestamp,
            current_timestamp: timestamp,
            params: market_params,
            hub_asset: hub_asset.clone(),
            cash: state.cash,
        }
    }

    pub fn save(&self) {
        let state = PoolStateRaw {
            supplied: self.supplied.raw(),
            borrowed: self.borrowed.raw(),
            revenue: self.revenue.raw(),
            borrow_index: self.borrow_index.raw(),
            supply_index: self.supply_index.raw(),
            last_timestamp: self.last_timestamp,
            cash: self.cash,
        };

        self.env
            .storage()
            .persistent()
            .set(&PoolKey::State(self.hub_asset.clone()), &state);
    }

    pub fn calculate_utilization(&self) -> Ray {
        if self.supplied == Ray::ZERO {
            return Ray::ZERO;
        }
        // dimensional: scaled shares times indexes become Ray<Token(asset)>.
        let total_borrowed = scaled_to_original(&self.env, self.borrowed, self.borrow_index);
        let total_supplied = scaled_to_original(&self.env, self.supplied, self.supply_index);

        rate_utilization(&self.env, total_borrowed, total_supplied)
    }

    pub fn require_reserves(&self, amount: i128) {
        assert_with_error!(
            self.env,
            self.cash >= amount,
            CollateralError::InsufficientLiquidity
        );
    }

    pub fn credit_cash(&mut self, amount: i128) {
        self.cash = self
            .cash
            .checked_add(amount)
            .unwrap_or_else(|| panic_with_error!(&self.env, GenericError::MathOverflow));
    }

    pub fn debit_cash(&mut self, amount: i128) {
        self.cash = self
            .cash
            .checked_sub(amount)
            .unwrap_or_else(|| panic_with_error!(&self.env, GenericError::MathOverflow));
    }

    pub fn transfer_out(&self, recipient: &Address, amount: i128) {
        if amount <= 0 {
            return;
        }
        let tok = token::Client::new(&self.env, &self.params.asset_id);
        tok.transfer(&self.env.current_contract_address(), recipient, &amount);
    }

    pub fn calculate_scaled_supply(&self, amount: i128) -> Ray {
        Ray::from_asset(amount, self.params.asset_decimals).div(&self.env, self.supply_index)
    }

    pub fn calculate_scaled_borrow(&self, amount: i128) -> Ray {
        Ray::from_asset(amount, self.params.asset_decimals).div(&self.env, self.borrow_index)
    }

    pub fn unscale_supply(&self, scaled: Ray) -> i128 {
        scaled_to_original(&self.env, scaled, self.supply_index)
            .to_asset(self.params.asset_decimals)
    }

    pub fn unscale_supply_floor(&self, scaled: Ray) -> i128 {
        scaled
            .mul_floor(&self.env, self.supply_index)
            .to_asset_floor(self.params.asset_decimals)
    }

    pub fn unscale_borrow(&self, scaled: Ray) -> i128 {
        scaled_to_original(&self.env, scaled, self.borrow_index)
            .to_asset(self.params.asset_decimals)
    }

    pub fn unscale_borrow_ceil(&self, scaled: Ray) -> i128 {
        scaled
            .mul(&self.env, self.borrow_index)
            .to_asset_ceil(self.params.asset_decimals)
    }

    pub fn unscale_borrow_exact(&self, scaled: Ray) -> Ray {
        scaled_to_original(&self.env, scaled, self.borrow_index)
    }

    /// Full-close when request ≥ half-up actual: burns all shares, pays floor gross.
    /// Controller dust gate MUST mirror this rule or position map and pool diverge.
    pub fn resolve_withdrawal(&self, amount: i128, pos_scaled: Ray) -> (Ray, i128) {
        let current_supply_actual = self.unscale_supply(pos_scaled);
        let current_supply_floor = self.unscale_supply_floor(pos_scaled);
        if amount >= current_supply_actual {
            return (pos_scaled, current_supply_floor);
        }
        (self.calculate_scaled_supply(amount), amount)
    }

    /// Floor conversion: a claim never transfers more than the shares it burns
    /// are worth, so rounding dust stays as supplier backing.
    pub fn burn_claimable_revenue(&mut self) -> i128 {
        let treasury_actual = self.unscale_supply_floor(self.revenue);
        let amount = self.cash.min(treasury_actual);
        if amount <= 0 {
            return 0;
        }
        let scaled_to_burn = if amount >= treasury_actual {
            self.revenue
        } else {
            let ratio = Ray::from_fraction(&self.env, amount, treasury_actual);
            self.revenue.mul(&self.env, ratio)
        };
        self.revenue.checked_sub_assign(&self.env, scaled_to_burn);
        self.supplied.checked_sub_assign(&self.env, scaled_to_burn);
        amount
    }

    pub fn resolve_repay(&self, amount: i128, pos_scaled: Ray) -> (Ray, i128) {
        let current_debt_ceil = self.unscale_borrow_ceil(pos_scaled);
        if amount >= current_debt_ceil {
            (
                pos_scaled,
                amount
                    .checked_sub(current_debt_ceil)
                    .unwrap_or_else(|| panic_with_error!(&self.env, GenericError::MathOverflow)),
            )
        } else {
            (self.calculate_scaled_borrow(amount), 0)
        }
    }

    pub fn market_index(&self) -> MarketIndexRaw {
        MarketIndexRaw {
            borrow_index: self.borrow_index.raw(),
            supply_index: self.supply_index.raw(),
        }
    }

    pub fn market_snapshot(&self) -> MarketStateSnapshot {
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

    pub fn position_mutation(&self, scaled: Ray, actual_amount: i128) -> PoolPositionMutation {
        PoolPositionMutation {
            position: ScaledPositionRaw {
                scaled_amount: scaled.raw(),
            },
            market_index: self.market_index(),
            actual_amount,
            asset_decimals: self.params.asset_decimals,
        }
    }

    pub fn strategy_mutation(
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

#[cfg(test)]
#[path = "../tests/cache.rs"]
mod tests;
