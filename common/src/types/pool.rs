use crate::constants::{BPS, MAX_BORROW_RATE_RAY, RAY, RAY_DECIMALS};
use crate::errors::CollateralError;
use crate::math::fp::{Bps, Ray};
use soroban_sdk::{assert_with_error, contracttype, panic_with_error, Address, Env};

/// Persistent pool parameter encoding.
///
/// `*_ray` fields use 27-decimal RAY scale, `*_bps` fields use basis points,
/// and `asset_decimals` is the SAC token decimal count used for conversions.
#[contracttype]
#[derive(Clone, Debug)]
pub struct MarketParamsRaw {
    pub max_borrow_rate_ray: i128,
    pub base_borrow_rate_ray: i128,
    pub slope1_ray: i128,
    pub slope2_ray: i128,
    pub slope3_ray: i128,
    pub mid_utilization_ray: i128,
    pub optimal_utilization_ray: i128,
    pub max_utilization_ray: i128,
    pub reserve_factor_bps: u32,
    /// Hub supply cap in asset-native units; zero or `i128::MAX` disables.
    pub supply_cap: i128,
    /// Hub borrow cap in asset-native units; zero or `i128::MAX` disables.
    pub borrow_cap: i128,
    pub asset_id: Address,
    pub asset_decimals: u32,
}

impl MarketParamsRaw {
    pub fn rate_model_view(&self) -> InterestRateModel {
        InterestRateModel {
            max_borrow_rate_ray: self.max_borrow_rate_ray,
            base_borrow_rate_ray: self.base_borrow_rate_ray,
            slope1_ray: self.slope1_ray,
            slope2_ray: self.slope2_ray,
            slope3_ray: self.slope3_ray,
            mid_utilization_ray: self.mid_utilization_ray,
            optimal_utilization_ray: self.optimal_utilization_ray,
            max_utilization_ray: self.max_utilization_ray,
            reserve_factor_bps: self.reserve_factor_bps,
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
    pub asset_id: Address,
    pub asset_decimals: u32,
}

impl From<&MarketParamsRaw> for MarketParams {
    fn from(r: &MarketParamsRaw) -> Self {
        Self {
            max_borrow_rate: Ray::from(r.max_borrow_rate_ray),
            base_borrow_rate: Ray::from(r.base_borrow_rate_ray),
            slope1: Ray::from(r.slope1_ray),
            slope2: Ray::from(r.slope2_ray),
            slope3: Ray::from(r.slope3_ray),
            mid_utilization: Ray::from(r.mid_utilization_ray),
            optimal_utilization: Ray::from(r.optimal_utilization_ray),
            max_utilization: Ray::from(r.max_utilization_ray),
            reserve_factor: Bps::from(i128::from(r.reserve_factor_bps)),
            supply_cap: r.supply_cap,
            borrow_cap: r.borrow_cap,
            asset_id: r.asset_id.clone(),
            asset_decimals: r.asset_decimals,
        }
    }
}

impl From<&MarketParams> for MarketParamsRaw {
    fn from(t: &MarketParams) -> Self {
        Self {
            max_borrow_rate_ray: t.max_borrow_rate.raw(),
            base_borrow_rate_ray: t.base_borrow_rate.raw(),
            slope1_ray: t.slope1.raw(),
            slope2_ray: t.slope2.raw(),
            slope3_ray: t.slope3.raw(),
            mid_utilization_ray: t.mid_utilization.raw(),
            optimal_utilization_ray: t.optimal_utilization.raw(),
            max_utilization_ray: t.max_utilization.raw(),
            reserve_factor_bps: t.reserve_factor.raw() as u32,
            supply_cap: t.supply_cap,
            borrow_cap: t.borrow_cap,
            asset_id: t.asset_id.clone(),
            asset_decimals: t.asset_decimals,
        }
    }
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct InterestRateModel {
    pub max_borrow_rate_ray: i128,
    pub base_borrow_rate_ray: i128,
    pub slope1_ray: i128,
    pub slope2_ray: i128,
    pub slope3_ray: i128,
    pub mid_utilization_ray: i128,
    pub optimal_utilization_ray: i128,
    pub max_utilization_ray: i128,
    pub reserve_factor_bps: u32,
}

impl InterestRateModel {
    pub fn verify(&self, env: &Env) {
        assert_with_error!(
            env,
            self.base_borrow_rate_ray >= 0,
            CollateralError::BaseRateNegative
        );
        if self.slope1_ray < self.base_borrow_rate_ray
            || self.slope2_ray < self.slope1_ray
            || self.slope3_ray < self.slope2_ray
            || self.max_borrow_rate_ray < self.slope3_ray
        {
            panic_with_error!(env, CollateralError::SlopeNonMonotonic);
        }
        assert_with_error!(
            env,
            self.max_borrow_rate_ray > self.base_borrow_rate_ray,
            CollateralError::MaxRateBelowBase
        );
        assert_with_error!(
            env,
            self.max_borrow_rate_ray <= MAX_BORROW_RATE_RAY,
            CollateralError::MaxBorrowRateTooHigh
        );
        assert_with_error!(
            env,
            self.mid_utilization_ray > 0,
            CollateralError::InvalidUtilRange
        );
        assert_with_error!(
            env,
            self.optimal_utilization_ray > self.mid_utilization_ray,
            CollateralError::InvalidUtilRange
        );
        assert_with_error!(
            env,
            self.optimal_utilization_ray < RAY,
            CollateralError::OptUtilTooHigh
        );
        if self.max_utilization_ray < self.optimal_utilization_ray || self.max_utilization_ray > RAY
        {
            panic_with_error!(env, CollateralError::InvalidUtilRange);
        }
        assert_with_error!(
            env,
            i128::from(self.reserve_factor_bps) < BPS,
            CollateralError::InvalidReserveFactor
        );
    }
}

/// Persistent collateral position encoding.
///
/// `scaled_amount_ray` is a supply share, not underlying balance.
/// Risk fields are snapshotted by the controller for HF/LTV/liquidation math.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountPositionRaw {
    pub scaled_amount_ray: i128,
    pub liquidation_threshold_bps: u32,
    pub liquidation_bonus_bps: u32,
    pub loan_to_value_bps: u32,
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
            scaled_amount: Ray::from(r.scaled_amount_ray),
            liquidation_threshold: Bps::from(i128::from(r.liquidation_threshold_bps)),
            liquidation_bonus: Bps::from(i128::from(r.liquidation_bonus_bps)),
            loan_to_value: Bps::from(i128::from(r.loan_to_value_bps)),
        }
    }
}

impl From<&AccountPosition> for AccountPositionRaw {
    fn from(t: &AccountPosition) -> Self {
        Self {
            scaled_amount_ray: t.scaled_amount.raw(),
            liquidation_threshold_bps: t.liquidation_threshold.raw() as u32,
            liquidation_bonus_bps: t.liquidation_bonus.raw() as u32,
            loan_to_value_bps: t.loan_to_value.raw() as u32,
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
    pub scaled_amount_ray: i128,
}

impl From<&AccountPosition> for ScaledPositionRaw {
    fn from(t: &AccountPosition) -> Self {
        Self {
            scaled_amount_ray: t.scaled_amount.raw(),
        }
    }
}

impl From<&DebtPosition> for ScaledPositionRaw {
    fn from(t: &DebtPosition) -> Self {
        Self {
            scaled_amount_ray: t.scaled_amount.raw(),
        }
    }
}

/// Persistent debt position encoding.
///
/// `scaled_amount_ray` is a borrow share, not underlying debt.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebtPositionRaw {
    pub scaled_amount_ray: i128,
}

/// Typed debt position used by borrow-index accounting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DebtPosition {
    pub scaled_amount: Ray,
}

impl From<&DebtPositionRaw> for DebtPosition {
    fn from(r: &DebtPositionRaw) -> Self {
        Self {
            scaled_amount: Ray::from(r.scaled_amount_ray),
        }
    }
}

// Pool returns the post-mutation scaled share, which is the full debt position.
impl From<&ScaledPositionRaw> for DebtPosition {
    fn from(r: &ScaledPositionRaw) -> Self {
        Self {
            scaled_amount: Ray::from(r.scaled_amount_ray),
        }
    }
}

impl From<&DebtPosition> for DebtPositionRaw {
    fn from(t: &DebtPosition) -> Self {
        Self {
            scaled_amount_ray: t.scaled_amount.raw(),
        }
    }
}

/// Borrow and supply indexes in RAY scale.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketIndexRaw {
    pub borrow_index_ray: i128,
    pub supply_index_ray: i128,
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
            borrow_index: Ray::from(r.borrow_index_ray),
            supply_index: Ray::from(r.supply_index_ray),
        }
    }
}

impl From<&MarketIndex> for MarketIndexRaw {
    fn from(t: &MarketIndex) -> Self {
        Self {
            borrow_index_ray: t.borrow_index.raw(),
            supply_index_ray: t.supply_index.raw(),
        }
    }
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct MarketStateSnapshot {
    /// Pool asset whose state was updated.
    pub asset: Address,
    /// Millisecond timestamp used for the accrual checkpoint.
    pub timestamp: u64,
    /// Supply index after accrual, in RAY.
    pub supply_index_ray: i128,
    /// Borrow index after accrual, in RAY.
    pub borrow_index_ray: i128,
    /// Pool token balance, in asset-native units.
    pub reserves_ray: i128,
    /// Total scaled supply shares.
    pub supplied_ray: i128,
    /// Total scaled borrow shares.
    pub borrowed_ray: i128,
    /// Scaled protocol revenue shares.
    pub revenue_ray: i128,
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

/// Persistent storage keys of the central pool, keyed by market asset.
#[contracttype]
#[derive(Clone, Debug)]
pub enum PoolKey {
    Params(Address),
    State(Address),
}

/// Asset-scoped mutation payload for the central pool ABI.
///
/// The funds counterparty (receiver/payer) is carried by endpoint arguments,
/// shared by each entry in a bulk call.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolAction {
    pub position: ScaledPositionRaw,
    pub amount: i128,
    pub asset: Address,
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
    pub supplied_ray: i128,
    pub borrowed_ray: i128,
    pub revenue_ray: i128,
    pub borrow_index_ray: i128,
    pub supply_index_ray: i128,
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
            supplied: Ray::from(r.supplied_ray),
            borrowed: Ray::from(r.borrowed_ray),
            revenue: Ray::from(r.revenue_ray),
            borrow_index: Ray::from(r.borrow_index_ray),
            supply_index: Ray::from(r.supply_index_ray),
            last_timestamp: r.last_timestamp,
            cash: r.cash,
        }
    }
}

impl From<&PoolState> for PoolStateRaw {
    fn from(t: &PoolState) -> Self {
        Self {
            supplied_ray: t.supplied.raw(),
            borrowed_ray: t.borrowed.raw(),
            revenue_ray: t.revenue.raw(),
            borrow_index_ray: t.borrow_index.raw(),
            supply_index_ray: t.supply_index.raw(),
            last_timestamp: t.last_timestamp,
            cash: t.cash,
        }
    }
}

#[cfg(test)]
#[path = "../../tests/types/pool.rs"]
mod tests;
