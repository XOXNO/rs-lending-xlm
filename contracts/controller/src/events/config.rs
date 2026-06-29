use soroban_sdk::{contractevent, contracttype, Address, BytesN};

use common::types::{SpokeAssetConfig, SpokeConfig};

use super::EventOracleProvider;

#[contractevent(topics = ["config", "oracle"])]
#[derive(Clone, Debug)]
pub struct UpdateAssetOracleEvent {
    pub asset: Address,
    pub oracle: EventOracleProvider,
}

/// Spoke snapshot emitted after spoke changes.
#[contracttype]
#[derive(Clone, Debug)]
pub struct EventSpoke {
    pub spoke_id: u32,
    pub is_deprecated: bool,
}

impl EventSpoke {
    pub fn new(spoke_id: u32, spoke: &SpokeConfig) -> Self {
        Self {
            spoke_id,
            is_deprecated: spoke.is_deprecated,
        }
    }
}

#[contractevent(topics = ["config", "spoke"])]
#[derive(Clone, Debug)]
pub struct UpdateSpokeEvent {
    pub spoke: EventSpoke,
}

#[contractevent(topics = ["config", "spoke_asset"])]
#[derive(Clone, Debug)]
pub struct UpdateSpokeAssetEvent {
    pub asset: Address,
    pub config: SpokeAssetConfig,
    pub spoke_id: u32,
    pub hub_id: u32,
}

#[contractevent(topics = ["config", "remove_spoke_asset"])]
#[derive(Clone, Debug)]
pub struct RemoveSpokeAssetEvent {
    pub asset: Address,
    pub spoke_id: u32,
    pub hub_id: u32,
}

#[contractevent(topics = ["config", "approve_token"])]
#[derive(Clone, Debug)]
pub struct ApproveTokenEvent {
    pub wasm_hash: BytesN<32>,
    pub approved: bool,
}

#[contractevent(topics = ["config", "approve_blend_pool"])]
#[derive(Clone, Debug)]
pub struct ApproveBlendPoolEvent {
    pub pool: Address,
    pub approved: bool,
}

#[contractevent(topics = ["config", "aggregator"])]
#[derive(Clone, Debug)]
pub struct UpdateAggregatorEvent {
    pub aggregator: Address,
}

#[contractevent(topics = ["config", "accumulator"])]
#[derive(Clone, Debug)]
pub struct UpdateAccumulatorEvent {
    pub accumulator: Address,
}

#[contractevent(topics = ["config", "pool_template"])]
#[derive(Clone, Debug)]
pub struct UpdatePoolTemplateEvent {
    pub wasm_hash: BytesN<32>,
}

#[contractevent(topics = ["config", "position_limits"])]
#[derive(Clone, Debug)]
pub struct UpdatePositionLimitsEvent {
    pub max_supply_positions: u32,
    pub max_borrow_positions: u32,
}

#[contractevent(topics = ["config", "min_borrow_collateral"])]
#[derive(Clone, Debug)]
pub struct UpdateMinBorrowCollateralEvent {
    pub min_borrow_collateral_usd_wad: i128,
}

#[contractevent(topics = ["config", "oracle_disabled"])]
#[derive(Clone, Debug)]
pub struct OracleDisabledEvent {
    pub asset: Address,
}

#[contractevent(topics = ["config", "hub"])]
#[derive(Clone, Debug)]
pub struct CreateHubEvent {
    pub hub_id: u32,
}
