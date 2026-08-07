use common::types::{InterestRateModel, MarketParamsRaw};
use soroban_sdk::{contractevent, Address};

/// Market-create event.
///
/// Rate-model fields are **flat** on the wire (not a nested `InterestRateModel`
/// / `MarketParamsRaw`). Flash-loan flags/fees and asset decimals stay off this
/// event; indexers that need them must read pool state or pool params events.
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
    /// Flatten create-market inputs onto the wire shape.
    ///
    /// Copies the shared interest-curve fields from `params` only; does **not**
    /// nest `MarketParamsRaw` or emit `is_flashloanable` / `flashloan_fee` /
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

/// Rate-model replace event.
///
/// Rate-model fields are **flat** on the wire (not a nested `InterestRateModel`).
/// Flash-loan flags/fees are intentionally omitted from this payload.
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
    /// Flatten an interest-rate model onto the wire shape for `(hub_id, asset)`.
    ///
    /// Copies the shared interest-curve fields only; does **not** nest
    /// `InterestRateModel` or emit `is_flashloanable` / `flashloan_fee`.
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

/// `(hub_id, asset, &InterestRateModel)` → flat event fields.
///
/// Pure `From<&InterestRateModel>` is impossible without dropping hub/asset; the
/// triple form keeps the mapping in one place while the published event stays
/// field-flat.
impl From<(u32, Address, &InterestRateModel)> for UpdateMarketParamsEvent {
    fn from((hub_id, asset, model): (u32, Address, &InterestRateModel)) -> Self {
        Self::from_rate_model(hub_id, asset, model)
    }
}
