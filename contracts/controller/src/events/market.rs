//! Market lifecycle and param-update events.

use soroban_sdk::{contractevent, Address};

#[contractevent(topics = ["market", "create"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateMarketEvent {
    pub hub_id: u32,
    pub base_asset: Address,
    pub max_borrow_rate: i128,
    pub base_borrow_rate: i128,
    pub slope1: i128,
    pub slope2: i128,
    pub slope3: i128,
    pub mid_utilization: i128,
    pub optimal_utilization: i128,
    pub max_utilization: i128,
    pub reserve_factor: u32,
    pub market_address: Address,
}

#[contractevent(topics = ["market", "params_update"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateMarketParamsEvent {
    pub asset: Address,
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
