//! Defines the contract events emitted when a market is created for a hub
//! asset and when a market's interest-rate parameters are replaced.

use common::types::{InterestRateModel, MarketParamsRaw};
use soroban_sdk::{contractevent, Address};

/// Event recording that a market is created for `hub_id` and `base_asset` at
/// `market_address`. Carries the interest-rate curve fields and reserve
/// factor as flat, top-level fields rather than a nested
/// `InterestRateModel`/`MarketParamsRaw`; omits the flash-loan flag, flash-loan
/// fee, and asset decimals.
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
    /// Builds a `CreateMarketEvent` for `hub_id`/`base_asset`/`market_address`,
    /// copying the interest-curve fields and reserve factor from `params`.
    /// Does not copy `is_flashloanable`, `flashloan_fee`, `asset_id`, or
    /// `asset_decimals`.
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

/// Event recording that the interest-rate model for `(hub_id, asset)` is
/// replaced. Carries the interest-rate curve fields and reserve factor as
/// flat, top-level fields rather than a nested `InterestRateModel`; omits the
/// flash-loan flag and flash-loan fee.
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
    /// Builds an `UpdateMarketParamsEvent` for `(hub_id, asset)`, copying the
    /// interest-curve fields and reserve factor from `model`. Does not copy
    /// `is_flashloanable` or `flashloan_fee`.
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

/// Converts a `(hub_id, asset, model)` triple into an `UpdateMarketParamsEvent`
/// by delegating to [`UpdateMarketParamsEvent::from_rate_model`].
impl From<(u32, Address, &InterestRateModel)> for UpdateMarketParamsEvent {
    /// Builds the event from the tuple's hub id, asset, and model by
    /// delegating to [`UpdateMarketParamsEvent::from_rate_model`].
    fn from((hub_id, asset, model): (u32, Address, &InterestRateModel)) -> Self {
        Self::from_rate_model(hub_id, asset, model)
    }
}
