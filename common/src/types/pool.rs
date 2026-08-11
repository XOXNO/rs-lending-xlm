//! Types owned by the per-hub-asset pool contract: interest-rate model parameters, market
//! state and indices (raw wire form and typed form), scaled position storage shapes, the
//! request/result types for each pool operation (supply, borrow, withdraw, strategy, seize,
//! net-settle), and the `PoolKey` storage key enum.

use crate::constants::{BPS, MAX_BORROW_RATE_RAY, MAX_FLASHLOAN_FEE_BPS, RAY, WAD_DECIMALS};
use crate::errors::CollateralError;
use crate::math::fp::{Bps, Ray};
use crate::types::shared::AccountPositionType;
use soroban_sdk::{assert_with_error, contracttype, panic_with_error, Address, Env};

/// Wire form of a market's interest-rate model and asset configuration, with rates as raw
/// ray-scaled `i128` values.
#[contracttype]
#[derive(Clone, Debug)]
pub struct MarketParamsRaw {
    pub max_borrow_rate: i128,

    pub base_borrow_rate: i128,

    pub slope1: i128,

    pub slope2: i128,

    pub slope3: i128,

    pub mid_utilization: i128,

    pub optimal_utilization: i128,

    pub max_utilization: i128,

    pub reserve_factor: u32,

    pub is_flashloanable: bool,

    pub flashloan_fee: u32,
    pub asset_id: Address,

    pub asset_decimals: u32,
}

impl MarketParamsRaw {
    /// Projects the rate-model fields out into a standalone `InterestRateModel`.
    pub fn rate_model_view(&self) -> InterestRateModel {
        InterestRateModel {
            max_borrow_rate: self.max_borrow_rate,
            base_borrow_rate: self.base_borrow_rate,
            slope1: self.slope1,
            slope2: self.slope2,
            slope3: self.slope3,
            mid_utilization: self.mid_utilization,
            optimal_utilization: self.optimal_utilization,
            max_utilization: self.max_utilization,
            reserve_factor: self.reserve_factor,
            is_flashloanable: self.is_flashloanable,
            flashloan_fee: self.flashloan_fee,
        }
    }

    /// Validates the rate-model fields via `InterestRateModel::verify`.
    pub fn verify_rate_model(&self, env: &Env) {
        self.rate_model_view().verify(env);
    }

    /// Validates `asset_decimals` and the rate model. Panics if `asset_decimals` exceeds
    /// `WAD_DECIMALS`, or if the rate model fails its own checks.
    pub fn verify(&self, env: &Env) {
        assert_with_error!(
            env,
            self.asset_decimals <= WAD_DECIMALS,
            CollateralError::AssetDecimalsTooHigh
        );

        self.verify_rate_model(env);
    }
}

/// Typed, in-memory form of `MarketParamsRaw`, with rates as `Ray`/`Bps` values.
#[derive(Clone, Debug)]
pub struct MarketParams {
    pub max_borrow_rate: Ray,
    pub base_borrow_rate: Ray,
    pub slope1: Ray,
    pub slope2: Ray,
    pub slope3: Ray,
    pub mid_utilization: Ray,
    pub optimal_utilization: Ray,
    pub max_utilization: Ray,
    pub reserve_factor: Bps,
    pub is_flashloanable: bool,

    pub flashloan_fee: u32,
    pub asset_id: Address,
    pub asset_decimals: u32,
}

impl From<&MarketParamsRaw> for MarketParams {
    fn from(r: &MarketParamsRaw) -> Self {
        Self {
            max_borrow_rate: Ray::from(r.max_borrow_rate),
            base_borrow_rate: Ray::from(r.base_borrow_rate),
            slope1: Ray::from(r.slope1),
            slope2: Ray::from(r.slope2),
            slope3: Ray::from(r.slope3),
            mid_utilization: Ray::from(r.mid_utilization),
            optimal_utilization: Ray::from(r.optimal_utilization),
            max_utilization: Ray::from(r.max_utilization),
            reserve_factor: Bps::from(i128::from(r.reserve_factor)),
            is_flashloanable: r.is_flashloanable,
            flashloan_fee: r.flashloan_fee,
            asset_id: r.asset_id.clone(),
            asset_decimals: r.asset_decimals,
        }
    }
}

impl From<&MarketParams> for MarketParamsRaw {
    fn from(t: &MarketParams) -> Self {
        Self {
            max_borrow_rate: t.max_borrow_rate.raw(),
            base_borrow_rate: t.base_borrow_rate.raw(),
            slope1: t.slope1.raw(),
            slope2: t.slope2.raw(),
            slope3: t.slope3.raw(),
            mid_utilization: t.mid_utilization.raw(),
            optimal_utilization: t.optimal_utilization.raw(),
            max_utilization: t.max_utilization.raw(),
            reserve_factor: t.reserve_factor.raw() as u32,
            is_flashloanable: t.is_flashloanable,
            flashloan_fee: t.flashloan_fee,
            asset_id: t.asset_id.clone(),
            asset_decimals: t.asset_decimals,
        }
    }
}

/// Standalone interest-rate-curve and flash-loan-fee configuration for a market, decoupled
/// from the asset identity fields carried by `MarketParamsRaw`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct InterestRateModel {
    pub max_borrow_rate: i128,

    pub base_borrow_rate: i128,
    pub slope1: i128,
    pub slope2: i128,
    pub slope3: i128,

    pub mid_utilization: i128,

    pub optimal_utilization: i128,

    pub max_utilization: i128,

    pub reserve_factor: u32,

    pub is_flashloanable: bool,

    pub flashloan_fee: u32,
}

impl InterestRateModel {
    /// Validates that the rate curve is well-formed: a non-negative base rate, non-decreasing
    /// slopes up through the max borrow rate, a max rate within bounds and above the base
    /// rate, an increasing and in-range utilization breakpoint sequence, a reserve factor
    /// below 100%, and a flashloan fee within the configured maximum. Panics if any of these
    /// checks fails.
    pub fn verify(&self, env: &Env) {
        assert_with_error!(
            env,
            self.base_borrow_rate >= 0,
            CollateralError::BaseRateNegative
        );
        if self.slope1 < self.base_borrow_rate
            || self.slope2 < self.slope1
            || self.slope3 < self.slope2
            || self.max_borrow_rate < self.slope3
        {
            panic_with_error!(env, CollateralError::SlopeNonMonotonic);
        }
        assert_with_error!(
            env,
            self.max_borrow_rate > self.base_borrow_rate,
            CollateralError::MaxRateBelowBase
        );
        assert_with_error!(
            env,
            self.max_borrow_rate <= MAX_BORROW_RATE_RAY,
            CollateralError::MaxBorrowRateTooHigh
        );
        assert_with_error!(
            env,
            self.mid_utilization > 0,
            CollateralError::InvalidUtilRange
        );
        assert_with_error!(
            env,
            self.optimal_utilization > self.mid_utilization,
            CollateralError::InvalidUtilRange
        );
        assert_with_error!(
            env,
            self.optimal_utilization < RAY,
            CollateralError::OptUtilTooHigh
        );
        if self.max_utilization < self.optimal_utilization || self.max_utilization > RAY {
            panic_with_error!(env, CollateralError::InvalidUtilRange);
        }
        assert_with_error!(
            env,
            i128::from(self.reserve_factor) < BPS,
            CollateralError::InvalidReserveFactor
        );
        assert_with_error!(
            env,
            i128::from(self.flashloan_fee) <= MAX_FLASHLOAN_FEE_BPS,
            CollateralError::InvalidBorrowParams
        );
    }
}

/// Wire form of a supply position: ray-scaled amount plus the risk parameters it was seeded
/// with at open (basis points as raw `u32`).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountPositionRaw {
    pub scaled_amount: i128,

    pub liquidation_threshold: u32,

    pub liquidation_bonus: u32,

    pub loan_to_value: u32,

    pub liquidation_fees: u32,
}

/// Typed, in-memory form of `AccountPositionRaw`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccountPosition {
    pub scaled_amount: Ray,
    pub liquidation_threshold: Bps,
    pub liquidation_bonus: Bps,
    pub loan_to_value: Bps,
    pub liquidation_fees: Bps,
}

impl From<&AccountPositionRaw> for AccountPosition {
    fn from(r: &AccountPositionRaw) -> Self {
        Self {
            scaled_amount: Ray::from(r.scaled_amount),
            liquidation_threshold: Bps::from(i128::from(r.liquidation_threshold)),
            liquidation_bonus: Bps::from(i128::from(r.liquidation_bonus)),
            loan_to_value: Bps::from(i128::from(r.loan_to_value)),
            liquidation_fees: Bps::from(i128::from(r.liquidation_fees)),
        }
    }
}

impl From<&AccountPosition> for AccountPositionRaw {
    fn from(t: &AccountPosition) -> Self {
        Self {
            scaled_amount: t.scaled_amount.raw(),
            liquidation_threshold: t.liquidation_threshold.raw() as u32,
            liquidation_bonus: t.liquidation_bonus.raw() as u32,
            loan_to_value: t.loan_to_value.raw() as u32,
            liquidation_fees: t.liquidation_fees.raw() as u32,
        }
    }
}

/// Wire form of a scaled position holding only the ray-scaled amount, without the risk
/// parameters carried by `AccountPositionRaw`. Used for debt positions and wherever a pool
/// operation only needs the scaled balance.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScaledPositionRaw {
    pub scaled_amount: i128,
}

impl From<&AccountPosition> for ScaledPositionRaw {
    fn from(t: &AccountPosition) -> Self {
        Self {
            scaled_amount: t.scaled_amount.raw(),
        }
    }
}

impl From<&DebtPosition> for ScaledPositionRaw {
    fn from(t: &DebtPosition) -> Self {
        Self {
            scaled_amount: t.scaled_amount.raw(),
        }
    }
}

/// Wire form of a debt position: the ray-scaled borrowed amount.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebtPositionRaw {
    pub scaled_amount: i128,
}

/// Typed, in-memory form of `DebtPositionRaw`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DebtPosition {
    pub scaled_amount: Ray,
}

impl From<&DebtPositionRaw> for DebtPosition {
    fn from(r: &DebtPositionRaw) -> Self {
        Self {
            scaled_amount: Ray::from(r.scaled_amount),
        }
    }
}

impl From<&ScaledPositionRaw> for DebtPosition {
    fn from(r: &ScaledPositionRaw) -> Self {
        Self {
            scaled_amount: Ray::from(r.scaled_amount),
        }
    }
}

impl From<&DebtPosition> for DebtPositionRaw {
    fn from(t: &DebtPosition) -> Self {
        Self {
            scaled_amount: t.scaled_amount.raw(),
        }
    }
}

/// Wire form of a market's cumulative borrow and supply interest indices, ray-scaled.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketIndexRaw {
    pub borrow_index: i128,

    pub supply_index: i128,
}

/// Typed, in-memory form of `MarketIndexRaw`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MarketIndex {
    pub borrow_index: Ray,
    pub supply_index: Ray,
}

impl From<&MarketIndexRaw> for MarketIndex {
    fn from(r: &MarketIndexRaw) -> Self {
        Self {
            borrow_index: Ray::from(r.borrow_index),
            supply_index: Ray::from(r.supply_index),
        }
    }
}

impl From<&MarketIndex> for MarketIndexRaw {
    fn from(t: &MarketIndex) -> Self {
        Self {
            borrow_index: t.borrow_index.raw(),
            supply_index: t.supply_index.raw(),
        }
    }
}

/// Point-in-time snapshot of a market's committed state, emitted after each pool mutation for
/// events and views: interest indices, cash on hand, total supplied and borrowed amounts, and
/// accrued protocol revenue.
#[contracttype]
#[derive(Clone, Debug)]
pub struct MarketStateSnapshot {
    pub hub_asset: HubAssetKey,

    pub timestamp: u64,

    pub supply_index: i128,

    pub borrow_index: i128,

    pub cash: i128,

    pub supplied: i128,

    pub borrowed: i128,

    pub revenue: i128,
}

/// Result of a supply, borrow, or withdraw mutation: the position's updated scaled amount,
/// the market's post-commit indices, the actual asset amount applied (gross, for withdraw),
/// and the asset's token decimals.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolPositionMutation {
    pub position: ScaledPositionRaw,
    pub market_index: MarketIndexRaw,

    pub actual_amount: i128,

    pub asset_decimals: u32,
}

/// Result of opening a strategy (leveraged) position: the updated debt position, the
/// market's post-commit indices, the principal amount borrowed (`actual_amount`), the net
/// amount actually transferred to the receiver after any fee (`amount_received`), and the
/// asset's token decimals.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolStrategyMutation {
    pub position: ScaledPositionRaw,
    pub market_index: MarketIndexRaw,

    pub actual_amount: i128,

    pub amount_received: i128,

    pub asset_decimals: u32,
}

/// Result of net-settling a user's supply against their debt on the same market: the
/// residual scaled supply and debt positions after burning the matched amount, the market's
/// post-commit indices, and the asset amount settled.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolNetSettleResult {
    pub supply_position: ScaledPositionRaw,
    pub debt_position: ScaledPositionRaw,
    pub market_index: MarketIndexRaw,

    pub settled_amount: i128,
}

impl From<&PoolStrategyMutation> for PoolPositionMutation {
    fn from(m: &PoolStrategyMutation) -> Self {
        Self {
            position: m.position.clone(),
            market_index: m.market_index.clone(),
            actual_amount: m.actual_amount,
            asset_decimals: m.asset_decimals,
        }
    }
}

/// Result of a plain amount-only pool mutation (recapitalize, claim revenue): the asset
/// amount actually applied.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolAmountMutation {
    pub actual_amount: i128,
}

/// A market's raw parameters and state loaded together, as returned by the pool's sync view.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolSyncData {
    pub params: MarketParamsRaw,
    pub state: PoolStateRaw,
}

/// Composite key identifying one asset's market within a hub.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HubAssetKey {
    pub hub_id: u32,
    pub asset: Address,
}

/// Storage keys for pool contract market state, keyed by `HubAssetKey`: `Params` for the
/// market's `MarketParamsRaw`, `State` for its `PoolStateRaw`.
#[contracttype]
#[derive(Clone, Debug)]
pub enum PoolKey {
    Params(HubAssetKey),
    State(HubAssetKey),
}

/// Common request shape for a pool operation on one market: the caller's current scaled
/// position, the requested asset amount, and the target market.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolAction {
    pub position: ScaledPositionRaw,

    pub amount: i128,
    pub hub_asset: HubAssetKey,
}

/// Request to supply assets into a market.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolSupplyEntry {
    pub action: PoolAction,
}

/// Request to borrow assets from a market.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolBorrowEntry {
    pub action: PoolAction,
}

/// Request to withdraw assets from a market, withholding `protocol_fee` from the gross
/// amount when the withdrawal is part of a liquidation.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolWithdrawEntry {
    pub action: PoolAction,

    pub protocol_fee: i128,
}

/// Request to seize a position during liquidation or bad-debt cleanup. `side` selects
/// whether the seized scaled amount is socialized as bad debt (`Borrow`) or absorbed as
/// protocol revenue (`Deposit`).
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolSeizeEntry {
    pub hub_asset: HubAssetKey,
    pub side: AccountPositionType,
    pub position: ScaledPositionRaw,
}

/// Request to net-settle a user's supply against their debt on the same market: the target
/// amount to settle, and the caller's current supply and debt positions.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolNetSettleEntry {
    pub hub_asset: HubAssetKey,

    pub amount: i128,
    pub supply_position: ScaledPositionRaw,
    pub debt_position: ScaledPositionRaw,
}

/// Wire form of a market's mutable state: totals, indices, cash, and last accrual timestamp.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolStateRaw {
    pub supplied: i128,

    pub borrowed: i128,

    pub revenue: i128,

    pub borrow_index: i128,

    pub supply_index: i128,
    pub last_timestamp: u64,

    pub cash: i128,
}

/// Typed, in-memory form of `PoolStateRaw`.
#[derive(Clone, Debug)]
pub struct PoolState {
    pub supplied: Ray,
    pub borrowed: Ray,
    pub revenue: Ray,
    pub borrow_index: Ray,
    pub supply_index: Ray,
    pub last_timestamp: u64,

    pub cash: i128,
}

impl From<&PoolStateRaw> for PoolState {
    fn from(r: &PoolStateRaw) -> Self {
        Self {
            supplied: Ray::from(r.supplied),
            borrowed: Ray::from(r.borrowed),
            revenue: Ray::from(r.revenue),
            borrow_index: Ray::from(r.borrow_index),
            supply_index: Ray::from(r.supply_index),
            last_timestamp: r.last_timestamp,
            cash: r.cash,
        }
    }
}

impl From<&PoolState> for PoolStateRaw {
    fn from(t: &PoolState) -> Self {
        Self {
            supplied: t.supplied.raw(),
            borrowed: t.borrowed.raw(),
            revenue: t.revenue.raw(),
            borrow_index: t.borrow_index.raw(),
            supply_index: t.supply_index.raw(),
            last_timestamp: t.last_timestamp,
            cash: t.cash,
        }
    }
}

#[cfg(test)]
#[path = "../../tests/types/pool.rs"]
mod tests;
