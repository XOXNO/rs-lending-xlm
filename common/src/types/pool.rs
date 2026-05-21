use crate::constants::{BPS, MAX_BORROW_RATE_RAY, RAY};
use crate::errors::CollateralError;
use crate::math::fp::{Bps, Ray};
use soroban_sdk::{contracttype, panic_with_error, Address, Env};

// Wire/storage form. Field names preserve the on-disk encoding.
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
}

// In-memory typed form. Used by every compute function.
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
    pub asset_id: Address,
    pub asset_decimals: u32,
}

impl From<&MarketParamsRaw> for MarketParams {
    fn from(r: &MarketParamsRaw) -> Self {
        Self {
            max_borrow_rate: Ray::from_raw(r.max_borrow_rate_ray),
            base_borrow_rate: Ray::from_raw(r.base_borrow_rate_ray),
            slope1: Ray::from_raw(r.slope1_ray),
            slope2: Ray::from_raw(r.slope2_ray),
            slope3: Ray::from_raw(r.slope3_ray),
            mid_utilization: Ray::from_raw(r.mid_utilization_ray),
            optimal_utilization: Ray::from_raw(r.optimal_utilization_ray),
            max_utilization: Ray::from_raw(r.max_utilization_ray),
            reserve_factor: Bps::from_raw(i128::from(r.reserve_factor_bps)),
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
        if self.base_borrow_rate_ray < 0
            || self.slope1_ray < self.base_borrow_rate_ray
            || self.slope2_ray < self.slope1_ray
            || self.slope3_ray < self.slope2_ray
            || self.max_borrow_rate_ray < self.slope3_ray
        {
            panic_with_error!(env, CollateralError::InvalidBorrowParams);
        }
        if self.max_borrow_rate_ray <= self.base_borrow_rate_ray {
            panic_with_error!(env, CollateralError::InvalidBorrowParams);
        }
        if self.max_borrow_rate_ray > MAX_BORROW_RATE_RAY {
            panic_with_error!(env, CollateralError::InvalidBorrowParams);
        }
        if self.mid_utilization_ray <= 0 {
            panic_with_error!(env, CollateralError::InvalidUtilRange);
        }
        if self.optimal_utilization_ray <= self.mid_utilization_ray {
            panic_with_error!(env, CollateralError::InvalidUtilRange);
        }
        if self.optimal_utilization_ray >= RAY {
            panic_with_error!(env, CollateralError::OptUtilTooHigh);
        }
        if self.max_utilization_ray < self.optimal_utilization_ray || self.max_utilization_ray > RAY
        {
            panic_with_error!(env, CollateralError::InvalidUtilRange);
        }
        if i128::from(self.reserve_factor_bps) >= BPS {
            panic_with_error!(env, CollateralError::InvalidReserveFactor);
        }
    }
}

// Wire/storage form. On-disk encoding: field-keyed map. Stored inside
// `Map<Address, AccountPositionRaw>` and crossed on the controller↔pool wire.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountPositionRaw {
    pub scaled_amount_ray: i128,
    pub liquidation_threshold_bps: u32,
    pub liquidation_bonus_bps: u32,
    pub liquidation_fees_bps: u32,
    pub loan_to_value_bps: u32,
}

// In-memory typed form. Used by every compute path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccountPosition {
    pub scaled_amount: Ray,
    pub liquidation_threshold: Bps,
    pub liquidation_bonus: Bps,
    pub liquidation_fees: Bps,
    pub loan_to_value: Bps,
}

impl From<&AccountPositionRaw> for AccountPosition {
    fn from(r: &AccountPositionRaw) -> Self {
        Self {
            scaled_amount: Ray::from_raw(r.scaled_amount_ray),
            liquidation_threshold: Bps::from_raw(i128::from(r.liquidation_threshold_bps)),
            liquidation_bonus: Bps::from_raw(i128::from(r.liquidation_bonus_bps)),
            liquidation_fees: Bps::from_raw(i128::from(r.liquidation_fees_bps)),
            loan_to_value: Bps::from_raw(i128::from(r.loan_to_value_bps)),
        }
    }
}

impl From<&AccountPosition> for AccountPositionRaw {
    fn from(t: &AccountPosition) -> Self {
        Self {
            scaled_amount_ray: t.scaled_amount.raw(),
            liquidation_threshold_bps: t.liquidation_threshold.raw() as u32,
            liquidation_bonus_bps: t.liquidation_bonus.raw() as u32,
            liquidation_fees_bps: t.liquidation_fees.raw() as u32,
            loan_to_value_bps: t.loan_to_value.raw() as u32,
        }
    }
}

// Wire/storage form. Embedded in PoolPositionMutation / PoolStrategyMutation /
// ControllerCache::market_indexes (Map values must be #[contracttype]).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketIndexRaw {
    pub borrow_index_ray: i128,
    pub supply_index_ray: i128,
}

// In-memory typed form. Used by every compute path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MarketIndex {
    pub borrow_index: Ray,
    pub supply_index: Ray,
}

impl From<&MarketIndexRaw> for MarketIndex {
    fn from(r: &MarketIndexRaw) -> Self {
        Self {
            borrow_index: Ray::from_raw(r.borrow_index_ray),
            supply_index: Ray::from_raw(r.supply_index_ray),
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
    pub asset: Address,
    pub timestamp: u64,
    pub supply_index_ray: i128,
    pub borrow_index_ray: i128,
    pub reserves_ray: i128,
    pub supplied_ray: i128,
    pub borrowed_ray: i128,
    pub revenue_ray: i128,
    pub asset_price_wad: Option<i128>,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolPositionMutation {
    pub position: AccountPositionRaw,
    pub market_index: MarketIndexRaw,
    pub market_state: MarketStateSnapshot,
    pub actual_amount: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolStrategyMutation {
    pub position: AccountPositionRaw,
    pub market_index: MarketIndexRaw,
    pub market_state: MarketStateSnapshot,
    pub actual_amount: i128,
    pub amount_received: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolAmountMutation {
    pub market_state: MarketStateSnapshot,
    pub actual_amount: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolSyncData {
    pub params: MarketParamsRaw,
    pub state: PoolStateRaw,
}

#[contracttype]
#[derive(Clone, Debug)]
pub enum PoolKey {
    Params,
    State,
}

// Wire/storage form.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolStateRaw {
    pub supplied_ray: i128,
    pub borrowed_ray: i128,
    pub revenue_ray: i128,
    pub borrow_index_ray: i128,
    pub supply_index_ray: i128,
    pub last_timestamp: u64,
}

// In-memory typed form.
#[derive(Clone, Debug)]
pub struct PoolState {
    pub supplied: Ray,
    pub borrowed: Ray,
    pub revenue: Ray,
    pub borrow_index: Ray,
    pub supply_index: Ray,
    pub last_timestamp: u64,
}

impl From<&PoolStateRaw> for PoolState {
    fn from(r: &PoolStateRaw) -> Self {
        Self {
            supplied: Ray::from_raw(r.supplied_ray),
            borrowed: Ray::from_raw(r.borrowed_ray),
            revenue: Ray::from_raw(r.revenue_ray),
            borrow_index: Ray::from_raw(r.borrow_index_ray),
            supply_index: Ray::from_raw(r.supply_index_ray),
            last_timestamp: r.last_timestamp,
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
        }
    }
}
