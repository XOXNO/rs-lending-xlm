//! In-memory market cache: loads params and interest state for one hub-asset,
//! runs scaled-share/reserve accounting and rounding conversions, and persists
//! the result. Rounding direction (half-up, floor, ceil) is chosen per flow to
//! keep protocol solvency conservative.

use common::errors::GenericError;
use common::math::fp::Ray;
use common::rates::scaled_to_original;
use common::types::{
    HubAssetKey, MarketIndexRaw, MarketParams, MarketParamsRaw, MarketStateSnapshot,
    PoolAmountMutation, PoolKey, PoolPositionMutation, PoolState, PoolStateRaw,
    PoolStrategyMutation, ScaledPositionRaw,
};
use soroban_sdk::{assert_with_error, panic_with_error, Env};

use crate::utils;

pub struct Cache {
    pub env: Env,
    // dimensional: Ray<Share(asset, supply)> total scaled supply.
    pub supplied: Ray,
    // dimensional: Ray<Share(asset, debt)> total scaled debt.
    pub borrowed: Ray,
    // dimensional: Ray<Share(asset, supply)> claimable protocol revenue.
    pub revenue: Ray,
    // dimensional: Ray<Index(asset, debt)> and Ray<Index(asset, supply)>.
    pub borrow_index: Ray,
    pub supply_index: Ray,
    pub last_timestamp: u64,
    pub current_timestamp: u64,
    pub params: MarketParams,
    /// Market key for cache loads and saves.
    pub hub_asset: HubAssetKey,
    // dimensional: Token(asset) tracked reserves; never Ray.
    pub cash: i128,
}

impl Cache {
    /// Loads market params and mutable interest state.
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
            hub_asset: hub_asset.clone(),
            cash: state.cash,
        }
    }

    /// Persists accrued market state.
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

    /// Utilization is borrowed / supplied; zero supply returns zero.
    pub fn calculate_utilization(&self) -> Ray {
        if self.supplied == Ray::ZERO {
            return Ray::ZERO;
        }
        // dimensional: scaled shares times indexes become Ray<Token(asset)>.
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

    /// Adds Token(asset) to tracked cash, panicking on overflow.
    pub fn credit_cash(&mut self, amount: i128) {
        self.cash = self
            .cash
            .checked_add(amount)
            .unwrap_or_else(|| panic_with_error!(&self.env, GenericError::MathOverflow));
    }

    /// Subtracts Token(asset) from tracked cash, panicking on under/overflow.
    pub fn debit_cash(&mut self, amount: i128) {
        self.cash = self
            .cash
            .checked_sub(amount)
            .unwrap_or_else(|| panic_with_error!(&self.env, GenericError::MathOverflow));
    }

    /// Transfers Token(asset) to `recipient`; zero and negative amounts are no-ops.
    pub fn transfer_out(&self, recipient: &soroban_sdk::Address, amount: i128) {
        if amount <= 0 {
            return;
        }
        let tok = soroban_sdk::token::Client::new(&self.env, &self.params.asset_id);
        tok.transfer(&self.env.current_contract_address(), recipient, &amount);
    }

    /// Converts an asset amount into scaled supply shares at the current index.
    pub fn calculate_scaled_supply(&self, amount: i128) -> Ray {
        // dimensional: Token(asset) / Ray<Index(asset, supply)> -> Ray<Share(asset, supply)>.
        Ray::from_asset(amount, self.params.asset_decimals).div(&self.env, self.supply_index)
    }

    /// Converts an asset amount into scaled debt shares at the current index.
    pub fn calculate_scaled_borrow(&self, amount: i128) -> Ray {
        // dimensional: Token(asset) / Ray<Index(asset, debt)> -> Ray<Share(asset, debt)>.
        Ray::from_asset(amount, self.params.asset_decimals).div(&self.env, self.borrow_index)
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
    pub fn unscale_borrow_exact(&self, scaled: Ray) -> Ray {
        scaled_to_original(&self.env, scaled, self.borrow_index)
    }

    /// Resolves withdrawal into burned supply shares and gross amount.
    ///
    /// # Notes
    /// Full-close quantization is a cross-contract contract: the position closes
    /// (all `pos_scaled` shares burned, floor-valued gross paid out) whenever the
    /// request meets or exceeds the half-up-rounded actual balance, or when the
    /// half-up remainder after a partial burn rounds to zero. The controller's
    /// dust gate MUST mirror this half-up full-close rule; if it decides "dust
    /// remains" while the pool full-closes (or vice versa), the position map and
    /// pool disagree and the withdrawal reverts.
    pub fn resolve_withdrawal(&self, amount: i128, pos_scaled: Ray) -> (Ray, i128) {
        // dimensional: returns Ray<Share(asset, supply)> burned and Token(asset) gross.
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

    /// Burns claimable revenue up to live reserves and returns the token amount.
    pub fn burn_claimable_revenue(&mut self) -> i128 {
        // dimensional: revenue is Ray<Share(asset, supply)>; transfer amount is Token(asset).
        let reserves = self.live_reserves();
        let treasury_actual = self.unscale_supply(self.revenue);
        let amount = reserves.min(treasury_actual);
        if amount <= 0 {
            return amount.max(0);
        }
        let scaled_to_burn = if amount >= treasury_actual {
            self.revenue
        } else {
            // dimensional: Token(asset) / Token(asset) -> Ray<1>.
            let ratio = Ray::from_fraction(&self.env, amount, treasury_actual);
            self.revenue.mul(&self.env, ratio)
        };
        // dimensional: burn same Ray<Share(asset, supply)> from revenue and total supply.
        self.revenue.checked_sub_assign(&self.env, scaled_to_burn);
        self.supplied.checked_sub_assign(&self.env, scaled_to_burn);
        amount
    }

    /// Resolves debt-share burn and overpayment refund.
    pub fn resolve_repay(&self, amount: i128, pos_scaled: Ray) -> (Ray, i128) {
        // dimensional: returns Ray<Share(asset, debt)> burned and Token(asset) refund.
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
            borrow_index: self.borrow_index.raw(),
            supply_index: self.supply_index.raw(),
        }
    }

    /// Snapshot emitted to indexers after each pool state mutation.
    pub fn market_snapshot(&self) -> MarketStateSnapshot {
        MarketStateSnapshot {
            hub_asset: self.hub_asset.clone(),
            timestamp: self.current_timestamp,
            supply_index: self.supply_index.raw(),
            borrow_index: self.borrow_index.raw(),
            // Asset-native cash, not a scaled RAY share like the sibling fields.
            cash: self.cash,
            supplied: self.supplied.raw(),
            borrowed: self.borrowed.raw(),
            revenue: self.revenue.raw(),
        }
    }

    /// Position mutation snapshot containing only the pool-owned scaled share.
    pub fn position_mutation(&self, scaled: Ray, actual_amount: i128) -> PoolPositionMutation {
        PoolPositionMutation {
            position: ScaledPositionRaw {
                // dimensional: Ray<Share(asset, side)> raw.
                scaled_amount: scaled.raw(),
            },
            market_index: self.market_index(),
            // dimensional: Token(asset) actual amount.
            actual_amount,
        }
    }

    /// Revenue claim mutation snapshot; actual amount is Token(asset).
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
                // dimensional: Ray<Share(asset, debt)> raw.
                scaled_amount: scaled.raw(),
            },
            market_index: self.market_index(),
            // dimensional: Token(asset) borrowed amount before fee.
            actual_amount,
            // dimensional: Token(asset) sent to caller after fee.
            amount_received,
        }
    }
}

#[cfg(test)]
#[path = "../tests/cache.rs"]
mod tests;
