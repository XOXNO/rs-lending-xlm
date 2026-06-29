use crate::constants::{BPS, MAX_BORROW_RATE_RAY, RAY, RAY_DECIMALS};
use crate::errors::CollateralError;
use crate::math::fp::{Bps, Ray};
use soroban_sdk::{assert_with_error, contracttype, panic_with_error, Address, Env};

/// Persistent pool parameter encoding.
///
/// Rate, index, and slope fields are RAY-scaled (27 decimals); ratio fields
/// (reserve factor, flashloan fee) are basis points. The scale is a convention,
/// not encoded in the field names. `asset_decimals` is the SAC token decimal
/// count used for conversions.
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
    /// Hub supply cap in asset-native units; zero or `i128::MAX` disables.
    pub supply_cap: i128,
    /// Hub borrow cap in asset-native units; zero or `i128::MAX` disables.
    pub borrow_cap: i128,
    /// Flash-loan eligibility; inert until Phase 2 wires the gate.
    pub is_flashloanable: bool,
    /// Flash-loan fee in bps; inert until Phase 2 wires the gate.
    pub flashloan_fee: u32,
    pub asset_id: Address,
    pub asset_decimals: u32,
}

impl MarketParamsRaw {
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
        }
    }

    pub fn verify_rate_model(&self, env: &Env) {
        self.rate_model_view().verify(env);
    }

    // Boundary validation: rate model plus `asset_decimals <= RAY_DECIMALS`
    // to keep `Ray::from_asset` inside the supported decimal domain.
    pub fn verify(&self, env: &Env) {
        assert_with_error!(
            env,
            self.asset_decimals <= RAY_DECIMALS,
            CollateralError::AssetDecimalsTooHigh
        );
        assert_with_error!(
            env,
            self.supply_cap >= 0 && self.borrow_cap >= 0,
            CollateralError::InvalidBorrowParams
        );
        // Hub caps share the e-mode spoke-cap domain guard: reject any cap that
        // would overflow `Ray::from_asset` during cap previews so a misconfig
        // fails here at the boundary, not as a runtime MathOverflow in a view.
        crate::validation::require_cap_within_asset_domain(
            env,
            self.supply_cap,
            self.asset_decimals,
        );
        crate::validation::require_cap_within_asset_domain(
            env,
            self.borrow_cap,
            self.asset_decimals,
        );
        self.verify_rate_model(env);
    }
}

/// Typed pool parameters used by interest and cap math.
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
    pub supply_cap: i128,
    pub borrow_cap: i128,
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
            supply_cap: r.supply_cap,
            borrow_cap: r.borrow_cap,
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
            supply_cap: t.supply_cap,
            borrow_cap: t.borrow_cap,
            is_flashloanable: t.is_flashloanable,
            flashloan_fee: t.flashloan_fee,
            asset_id: t.asset_id.clone(),
            asset_decimals: t.asset_decimals,
        }
    }
}

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
}

impl InterestRateModel {
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
        if self.max_utilization < self.optimal_utilization || self.max_utilization > RAY
        {
            panic_with_error!(env, CollateralError::InvalidUtilRange);
        }
        assert_with_error!(
            env,
            i128::from(self.reserve_factor) < BPS,
            CollateralError::InvalidReserveFactor
        );
    }
}

/// Persistent collateral position encoding.
///
/// `scaled_amount` is a supply share, not underlying balance.
/// Risk fields are snapshotted by the controller for HF/LTV/liquidation math.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountPositionRaw {
    pub scaled_amount: i128,
    pub liquidation_threshold: u32,
    pub liquidation_bonus: u32,
    pub loan_to_value: u32,
}

/// Typed collateral position used by controller risk math.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccountPosition {
    pub scaled_amount: Ray,
    pub liquidation_threshold: Bps,
    pub liquidation_bonus: Bps,
    pub loan_to_value: Bps,
}

impl From<&AccountPositionRaw> for AccountPosition {
    fn from(r: &AccountPositionRaw) -> Self {
        Self {
            scaled_amount: Ray::from(r.scaled_amount),
            liquidation_threshold: Bps::from(i128::from(r.liquidation_threshold)),
            liquidation_bonus: Bps::from(i128::from(r.liquidation_bonus)),
            loan_to_value: Bps::from(i128::from(r.loan_to_value)),
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
        }
    }
}

/// Pool ABI position shape containing only scaled shares.
///
/// Collateral risk parameters stay on the controller and do not cross the pool
/// boundary.
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

/// Persistent debt position encoding.
///
/// `scaled_amount` is a borrow share, not underlying debt.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebtPositionRaw {
    pub scaled_amount: i128,
}

/// Typed debt position used by borrow-index accounting.
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

// Pool returns the post-mutation scaled share, which is the full debt position.
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

/// Borrow and supply indexes in RAY scale.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketIndexRaw {
    pub borrow_index: i128,
    pub supply_index: i128,
}

/// Typed borrow and supply indexes.
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

#[contracttype]
#[derive(Clone, Debug)]
pub struct MarketStateSnapshot {
    /// Hub-asset coordinate whose state was updated.
    pub hub_asset: HubAssetKey,
    /// Millisecond timestamp used for the accrual checkpoint.
    pub timestamp: u64,
    /// Supply index after accrual, in RAY.
    pub supply_index: i128,
    /// Borrow index after accrual, in RAY.
    pub borrow_index: i128,
    /// Pool token balance, in asset-native units.
    pub cash: i128,
    /// Total scaled supply shares.
    pub supplied: i128,
    /// Total scaled borrow shares.
    pub borrowed: i128,
    /// Scaled protocol revenue shares.
    pub revenue: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolPositionMutation {
    pub position: ScaledPositionRaw,
    pub market_index: MarketIndexRaw,
    pub actual_amount: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolStrategyMutation {
    pub position: ScaledPositionRaw,
    pub market_index: MarketIndexRaw,
    pub actual_amount: i128,
    pub amount_received: i128,
}

impl From<&PoolStrategyMutation> for PoolPositionMutation {
    fn from(m: &PoolStrategyMutation) -> Self {
        Self {
            position: m.position.clone(),
            market_index: m.market_index.clone(),
            actual_amount: m.actual_amount,
        }
    }
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolAmountMutation {
    pub actual_amount: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolSyncData {
    pub params: MarketParamsRaw,
    pub state: PoolStateRaw,
}

/// Coordinate addressing one asset's liquidity within a specific hub.
///
/// `hub_id` namespaces isolated liquidity; the same `asset` on two hubs is two
/// independent markets that never net or cross-socialize.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HubAssetKey {
    pub hub_id: u32,
    pub asset: Address,
}

/// Persistent storage keys of the central pool, keyed by hub-asset coordinate.
#[contracttype]
#[derive(Clone, Debug)]
pub enum PoolKey {
    Params(HubAssetKey),
    State(HubAssetKey),
}

/// Hub-asset-scoped mutation payload for the central pool ABI.
///
/// The funds counterparty (receiver/payer) is carried by endpoint arguments,
/// shared by each entry in a bulk call.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolAction {
    pub position: ScaledPositionRaw,
    pub amount: i128,
    pub hub_asset: HubAssetKey,
}

/// One asset of a bulk pool `supply`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolSupplyEntry {
    pub action: PoolAction,
}

/// One asset of a bulk pool `borrow`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolBorrowEntry {
    pub action: PoolAction,
}

/// One asset of a bulk pool `withdraw`. The liquidation flag is per call;
/// the protocol fee scales with each asset's value and stays per entry.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolWithdrawEntry {
    pub action: PoolAction,
    pub protocol_fee: i128,
}

/// Persistent pool accounting state.
///
/// Supply, borrow, and revenue totals are scaled shares; indexes convert them
/// to underlying amounts.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolStateRaw {
    pub supplied: i128,
    pub borrowed: i128,
    pub revenue: i128,
    pub borrow_index: i128,
    pub supply_index: i128,
    pub last_timestamp: u64,
    /// Liquid token units the pool holds (available reserves), tracked internally
    /// on each in/out flow instead of reading the token balance. Direct donations
    /// cannot inflate borrowable liquidity.
    pub cash: i128,
}

/// Typed pool accounting state.
#[derive(Clone, Debug)]
pub struct PoolState {
    pub supplied: Ray,
    pub borrowed: Ray,
    pub revenue: Ray,
    pub borrow_index: Ray,
    pub supply_index: Ray,
    pub last_timestamp: u64,
    /// Liquid token units held by the pool (available reserves).
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
