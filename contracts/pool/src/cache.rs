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

/// In-memory representation of a market's params + mutable interest state.
/// Used to batch accrual, accounting mutations, and a single save at the end
/// of each high-level operation.
pub struct Cache {
    /// Contract environment handle.
    pub env: Env,
    /// Total scaled supply shares — `Ray<Share(asset, supply)>`.
    pub supplied: Ray,
    /// Total scaled debt shares — `Ray<Share(asset, debt)>`.
    pub borrowed: Ray,
    /// Claimable protocol revenue, held as supply shares — `Ray<Share(asset, supply)>`.
    pub revenue: Ray,
    /// Debt interest index — `Ray<Index(asset, debt)>`.
    pub borrow_index: Ray,
    /// Supply interest index — `Ray<Index(asset, supply)>`.
    pub supply_index: Ray,
    /// Last accrual checkpoint, in milliseconds.
    pub last_timestamp: u64,
    /// Current ledger time, in milliseconds.
    pub current_timestamp: u64,
    /// Market interest-rate and risk parameters.
    pub params: MarketParams,
    /// Market key for cache loads and saves.
    pub hub_asset: HubAssetKey,
    /// Tracked reserves in asset-native units (`Token(asset)`, never Ray).
    pub cash: i128,
}

impl Cache {
    /// Loads market params and mutable interest state.
    ///
    /// # Arguments
    /// * `env` - Soroban environment.
    /// * `hub_asset` - the market identifier.
    ///
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

    /// Persists accrued market state.
    ///
    /// # Notes
    /// Only state is written; params are immutable after market creation.
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

    // ################## QUERY STATE ##################

    /// Utilization is borrowed / supplied; zero supply returns zero.
    pub fn calculate_utilization(&self) -> Ray {
        if self.supplied == Ray::ZERO {
            return Ray::ZERO;
        }
        // dimensional: scaled shares times indexes become Ray<Token(asset)>.
        let total_borrowed = scaled_to_original(&self.env, self.borrowed, self.borrow_index);
        let total_supplied = scaled_to_original(&self.env, self.supplied, self.supply_index);

        rate_utilization(&self.env, total_borrowed, total_supplied)
    }

    /// Panics with `InsufficientLiquidity` if tracked cash is below `amount`.
    pub fn require_reserves(&self, amount: i128) {
        assert_with_error!(
            self.env,
            self.cash >= amount,
            CollateralError::InsufficientLiquidity
        );
    }

    // ################## CHANGE STATE ##################

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
    pub fn transfer_out(&self, recipient: &Address, amount: i128) {
        if amount <= 0 {
            return;
        }
        let tok = token::Client::new(&self.env, &self.params.asset_id);
        tok.transfer(&self.env.current_contract_address(), recipient, &amount);
    }

    // ################## LOW-LEVEL HELPERS ##################

    /// Converts an asset amount into scaled supply shares at the current index.
    pub fn calculate_scaled_supply(&self, amount: i128) -> Ray {
        Ray::from_asset(amount, self.params.asset_decimals).div(&self.env, self.supply_index)
    }

    /// Converts an asset amount into scaled debt shares at the current index.
    pub fn calculate_scaled_borrow(&self, amount: i128) -> Ray {
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
    /// request meets or exceeds the half-up-rounded actual balance. The
    /// controller's dust gate MUST mirror this full-close rule; if it decides
    /// "dust remains" while the pool full-closes (or vice versa), the position
    /// map and pool disagree and the withdrawal reverts.
    pub fn resolve_withdrawal(&self, amount: i128, pos_scaled: Ray) -> (Ray, i128) {
        let current_supply_actual = self.unscale_supply(pos_scaled);
        let current_supply_floor = self.unscale_supply_floor(pos_scaled);
        if amount >= current_supply_actual {
            return (pos_scaled, current_supply_floor);
        }
        (self.calculate_scaled_supply(amount), amount)
    }

    /// Burns claimable revenue up to tracked cash and returns the token amount.
    pub fn burn_claimable_revenue(&mut self) -> i128 {
        let treasury_actual = self.unscale_supply(self.revenue);
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

    /// Resolves debt-share burn and overpayment refund.
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
                scaled_amount: scaled.raw(),
            },
            market_index: self.market_index(),
            actual_amount,
        }
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
                scaled_amount: scaled.raw(),
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
