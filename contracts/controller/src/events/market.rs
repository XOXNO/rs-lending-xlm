//! Events for market creation and interest-rate updates.

use common::types::{InterestRateModel, MarketParamsRaw};
use soroban_sdk::{contractevent, Address};

/// Emitted when a market is created for `(hub_id, base_asset)` at `market_address`. The event
/// flattens the curve fields and reserve factor. It omits the flash-loan settings and asset
/// decimals.
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

impl CreateMarketEvent {
    /// Copies curve fields and reserve factor and uses the supplied market identity.
    pub fn from_params(
        hub_id: u32,
        base_asset: Address,
        market_address: Address,
        params: &MarketParamsRaw,
    ) -> Self {
        Self {
            hub_id,
            base_asset,
            max_borrow_rate: params.max_borrow_rate,
            base_borrow_rate: params.base_borrow_rate,
            slope1: params.slope1,
            slope2: params.slope2,
            slope3: params.slope3,
            mid_utilization: params.mid_utilization,
            optimal_utilization: params.optimal_utilization,
            max_utilization: params.max_utilization,
            reserve_factor: params.reserve_factor,
            market_address,
        }
    }
}

/// Emitted when the rate model is replaced for `(hub_id, asset)`. The event flattens the curve
/// fields and reserve factor. It omits the flash-loan settings.
#[contractevent(topics = ["market", "params_update"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateMarketParamsEvent {
    pub hub_id: u32,
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

impl UpdateMarketParamsEvent {
    /// Copies the model's curve fields and reserve factor.
    pub fn from_rate_model(hub_id: u32, asset: Address, model: &InterestRateModel) -> Self {
        Self {
            hub_id,
            asset,
            max_borrow_rate: model.max_borrow_rate,
            base_borrow_rate: model.base_borrow_rate,
            slope1: model.slope1,
            slope2: model.slope2,
            slope3: model.slope3,
            mid_utilization: model.mid_utilization,
            optimal_utilization: model.optimal_utilization,
            max_utilization: model.max_utilization,
            reserve_factor: model.reserve_factor,
        }
    }
}
