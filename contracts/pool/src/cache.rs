use common::errors::GenericError;
use common::math::fp::Ray;
use common::rates::scaled_to_original;
use common::types::{
    MarketIndexRaw, MarketParams, MarketParamsRaw, MarketStateSnapshot, PoolAmountMutation,
    PoolKey, PoolPositionMutation, PoolState, PoolStateRaw, PoolStrategyMutation,
    ScaledPositionRaw,
};
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env};

use crate::utils;

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
    pub cash: i128,
}

impl Cache {
    /// Loads the market's params and mutable interest state for `asset` from
    /// persistent storage. Panics with PoolNotInitialized if either record is
    /// absent.
    pub fn load(env: &Env, asset: &Address) -> Self {
        let params: MarketParamsRaw = env
            .storage()
            .persistent()
            .get(&PoolKey::Params(asset.clone()))
            .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized));

        let raw_state: PoolStateRaw = env
            .storage()
            .persistent()
            .get(&PoolKey::State(asset.clone()))
            .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized));
        // Renew after successful loads; `extend_ttl` requires existing keys.
        utils::renew_market_keys(env, asset);
        let state = PoolState::from(&raw_state);
        let market_params = MarketParams::from(&params);
        let timestamp = utils::now_ms(env);

        Cache {
            env: env.clone(),
            supplied: state.supplied,
            borrowed: state.borrowed,
            revenue: state.revenue,
            borrow_index: state.borrow_index,
            supply_index: state.supply_index,
            last_timestamp: state.last_timestamp,
            current_timestamp: timestamp,
            params: market_params,
            cash: state.cash,
        }
    }

    /// Persists the current interest state (indexes, supplied/borrowed totals,
    /// revenue, last accrual timestamp) back to the asset-keyed persistent slot.
    pub fn save(&self) {
        let state = PoolStateRaw {
            supplied_ray: self.supplied.raw(),
            borrowed_ray: self.borrowed.raw(),
            revenue_ray: self.revenue.raw(),
            borrow_index_ray: self.borrow_index.raw(),
            supply_index_ray: self.supply_index.raw(),
            last_timestamp: self.last_timestamp,
            cash: self.cash,
        };

        self.env
            .storage()
            .persistent()
            .set(&PoolKey::State(self.params.asset_id.clone()), &state);
    }

    /// Current utilization = total_borrowed_value / total_supplied_value (RAY).
    /// Returns zero when supplied is zero (avoids div-by-zero).
    pub fn calculate_utilization(&self) -> Ray {
        if self.supplied == Ray::ZERO {
            return Ray::ZERO;
        }
        let total_borrowed = scaled_to_original(&self.env, self.borrowed, self.borrow_index);
        let total_supplied = scaled_to_original(&self.env, self.supplied, self.supply_index);

        common::rates::utilization(&self.env, total_borrowed, total_supplied)
    }

    /// Returns true when available reserves are at least `amount`.
    pub fn has_reserves(&self, amount: i128) -> bool {
        let reserves = self.live_reserves();
        reserves >= amount
    }

    /// Panics with InsufficientLiquidity if available reserves < amount.
    pub fn require_reserves(&self, amount: i128) {
        assert_with_error!(
            self.env,
            self.has_reserves(amount),
            common::errors::CollateralError::InsufficientLiquidity
        )
    }

    /// Available reserves are tracked `cash`; direct token donations do not
    /// increase borrowable liquidity, and no token balance call is needed.
    pub fn live_reserves(&self) -> i128 {
        self.cash
    }

    /// Adds `amount` to tracked cash, panicking on overflow.
    pub fn credit_cash(&mut self, amount: i128) {
        self.cash = self
            .cash
            .checked_add(amount)
            .unwrap_or_else(|| panic_with_error!(&self.env, GenericError::MathOverflow));
    }

    /// Subtracts `amount` from tracked cash, panicking on under/overflow.
    pub fn debit_cash(&mut self, amount: i128) {
        self.cash = self
            .cash
            .checked_sub(amount)
            .unwrap_or_else(|| panic_with_error!(&self.env, GenericError::MathOverflow));
    }

    /// Transfers pool asset to `recipient`; zero and negative amounts are no-ops.
    pub fn transfer_out(&self, recipient: &soroban_sdk::Address, amount: i128) {
        if amount <= 0 {
            return;
        }
        let tok = soroban_sdk::token::Client::new(&self.env, &self.params.asset_id);
        tok.transfer(&self.env.current_contract_address(), recipient, &amount);
    }

    /// Converts an asset amount into scaled supply shares at the current index.
    pub fn calculate_scaled_supply(&self, amount: i128) -> Ray {
        let amount_ray = Ray::from_asset(amount, self.params.asset_decimals);
        amount_ray.div(&self.env, self.supply_index)
    }

    /// Converts an asset amount into scaled debt shares at the current index.
    pub fn calculate_scaled_borrow(&self, amount: i128) -> Ray {
        let amount_ray = Ray::from_asset(amount, self.params.asset_decimals);
        amount_ray.div(&self.env, self.borrow_index)
    }

    /// Converts scaled supply shares to asset units using half-up rounding.
    pub fn unscale_supply(&self, scaled: Ray) -> i128 {
        scaled_to_original(&self.env, scaled, self.supply_index)
            .to_asset(self.params.asset_decimals)
    }

    /// Converts supply shares to asset units rounded down for user credits.
    pub fn unscale_supply_floor(&self, scaled: Ray) -> i128 {
        scaled
            .mul_floor(&self.env, self.supply_index)
            .to_asset_floor(self.params.asset_decimals)
    }

    /// Converts scaled debt shares to asset units using half-up rounding.
    pub fn unscale_borrow(&self, scaled: Ray) -> i128 {
        scaled_to_original(&self.env, scaled, self.borrow_index)
            .to_asset(self.params.asset_decimals)
    }

    /// Converts debt shares to asset units rounded up for user debits.
    pub fn unscale_borrow_ceil(&self, scaled: Ray) -> i128 {
        scaled
            .mul(&self.env, self.borrow_index)
            .to_asset_ceil(self.params.asset_decimals)
    }

    /// Converts scaled debt shares to underlying debt in RAY.
    pub fn unscale_borrow_ray(&self, scaled: Ray) -> Ray {
        scaled_to_original(&self.env, scaled, self.borrow_index)
    }

    /// Resolves a withdrawal into scaled shares and gross asset amount.
    ///
    /// Full-close floor rounding prevents over-crediting on the final scaled
    /// supply share.
    pub fn resolve_withdrawal(&self, amount: i128, pos_scaled: Ray) -> (Ray, i128) {
        let current_supply_actual = self.unscale_supply(pos_scaled);
        let current_supply_floor = self.unscale_supply_floor(pos_scaled);
        if amount >= current_supply_actual {
            return (pos_scaled, current_supply_floor);
        }
        let scaled = self.calculate_scaled_supply(amount);
        let remaining_actual = self.unscale_supply(pos_scaled - scaled);
        if remaining_actual == 0 {
            (pos_scaled, current_supply_floor)
        } else {
            (scaled, amount)
        }
    }

    /// Burns claimable revenue shares, capped by live reserves and scaled revenue.
    pub fn burn_claimable_revenue(&mut self) -> i128 {
        let reserves = self.live_reserves();
        let treasury_actual = self.unscale_supply(self.revenue);
        let amount = reserves.min(treasury_actual);
        if amount <= 0 {
            return amount.max(0);
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

    /// Resolves repayment into debt shares and overpayment refund.
    ///
    /// Full-close uses ceiling rounding so repayment cannot leave indexed dust.
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

    /// Current borrow and supply indexes in event/wire form.
    pub fn market_index(&self) -> MarketIndexRaw {
        MarketIndexRaw {
            borrow_index_ray: self.borrow_index.raw(),
            supply_index_ray: self.supply_index.raw(),
        }
    }

    /// Snapshot emitted to indexers after each pool state mutation.
    pub fn market_snapshot(&self) -> MarketStateSnapshot {
        MarketStateSnapshot {
            asset: self.params.asset_id.clone(),
            timestamp: self.current_timestamp,
            supply_index_ray: self.supply_index.raw(),
            borrow_index_ray: self.borrow_index.raw(),
            // Carries asset-native `cash`, not a RAY value; field name is wire ABI.
            reserves_ray: self.cash,
            supplied_ray: self.supplied.raw(),
            borrowed_ray: self.borrowed.raw(),
            revenue_ray: self.revenue.raw(),
        }
    }

    /// Position mutation snapshot containing only the pool-owned scaled share.
    pub fn position_mutation(&self, scaled: Ray, actual_amount: i128) -> PoolPositionMutation {
        PoolPositionMutation {
            position: ScaledPositionRaw {
                scaled_amount_ray: scaled.raw(),
            },
            market_index: self.market_index(),
            actual_amount,
        }
    }

    /// Revenue claim mutation snapshot.
    pub fn amount_mutation(&self, actual_amount: i128) -> PoolAmountMutation {
        PoolAmountMutation { actual_amount }
    }

    /// Strategy borrow mutation snapshot, including net amount sent to caller.
    pub fn strategy_mutation(
        &self,
        scaled: Ray,
        actual_amount: i128,
        amount_received: i128,
    ) -> PoolStrategyMutation {
        PoolStrategyMutation {
            position: ScaledPositionRaw {
                scaled_amount_ray: scaled.raw(),
            },
            market_index: self.market_index(),
            actual_amount,
            amount_received,
        }
    }
}

#[cfg(test)]
#[path = "../tests/cache.rs"]
mod tests;
