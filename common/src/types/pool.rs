use crate::constants::{BPS, MAX_BORROW_RATE_RAY, RAY};
use crate::errors::CollateralError;
use soroban_sdk::{contracttype, panic_with_error, Address, Env};

#[contracttype]
#[derive(Clone, Debug)]
pub struct MarketParams {
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

impl MarketParams {
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

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountPosition {
    pub scaled_amount_ray: i128,
    pub liquidation_threshold_bps: u32,
    pub liquidation_bonus_bps: u32,
    pub liquidation_fees_bps: u32,
    pub loan_to_value_bps: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketIndex {
    pub borrow_index_ray: i128,
    pub supply_index_ray: i128,
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
    pub position: AccountPosition,
    pub market_index: MarketIndex,
    pub market_state: MarketStateSnapshot,
    pub actual_amount: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolStrategyMutation {
    pub position: AccountPosition,
    pub market_index: MarketIndex,
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
    pub params: MarketParams,
    pub state: PoolState,
}

#[contracttype]
#[derive(Clone, Debug)]
pub enum PoolKey {
    Params,
    State,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolState {
    pub supplied_ray: i128,
    pub borrowed_ray: i128,
    pub revenue_ray: i128,
    pub borrow_index_ray: i128,
    pub supply_index_ray: i128,
    pub last_timestamp: u64,
}
