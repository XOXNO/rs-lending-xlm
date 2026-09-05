//! Events for controller configuration changes.

use soroban_sdk::{contractevent, contracttype, Address};

use common::types::{SpokeAssetConfig, SpokeConfig};

/// Spoke deprecation and liquidation-curve snapshot for [`UpdateSpokeEvent`].
#[contracttype]
#[derive(Clone, Debug)]
pub struct EventSpoke {
    pub spoke_id: u32,
    pub is_deprecated: bool,
    pub liquidation_target_hf_wad: i128,
    pub hf_for_max_bonus_wad: i128,
    pub liquidation_bonus_factor_bps: u32,
}

impl EventSpoke {
    /// Copies the spoke's deprecation flag and liquidation curve.
    pub fn new(spoke_id: u32, spoke: &SpokeConfig) -> Self {
        Self {
            spoke_id,
            is_deprecated: spoke.is_deprecated,
            liquidation_target_hf_wad: spoke.liquidation_target_hf_wad,
            hf_for_max_bonus_wad: spoke.hf_for_max_bonus_wad,
            liquidation_bonus_factor_bps: spoke.liquidation_bonus_factor_bps,
        }
    }
}

/// Spoke configuration created or updated.
#[contractevent(topics = ["config", "spoke"])]
#[derive(Clone, Debug)]
pub struct UpdateSpokeEvent {
    pub spoke: EventSpoke,
}

/// Spoke asset configuration created or updated.
#[contractevent(topics = ["config", "spoke_asset"])]
#[derive(Clone, Debug)]
pub struct UpdateSpokeAssetEvent {
    pub asset: Address,
    pub config: SpokeAssetConfig,
    pub spoke_id: u32,
    pub hub_id: u32,
}

/// Spoke asset configuration removed.
#[contractevent(topics = ["config", "remove_spoke_asset"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoveSpokeAssetEvent {
    pub asset: Address,
    pub spoke_id: u32,
    pub hub_id: u32,
}

/// Blend migration source approval changed.
#[contractevent(topics = ["config", "approve_blend_pool"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApproveBlendPoolEvent {
    pub pool: Address,
    pub approved: bool,
}

/// Swap aggregator address changed.
#[contractevent(topics = ["config", "swap_aggregator"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateSwapAggregatorEvent {
    pub swap_aggregator: Address,
}

/// Price aggregator address changed.
#[contractevent(topics = ["config", "price_aggregator"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdatePriceAggregatorEvent {
    pub price_aggregator: Address,
}

/// Revenue accumulator address changed.
#[contractevent(topics = ["config", "accumulator"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateAccumulatorEvent {
    pub accumulator: Address,
}

/// Per-account supply and borrow position limits changed.
#[contractevent(topics = ["config", "position_limits"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdatePositionLimitsEvent {
    pub max_supply_positions: u32,
    pub max_borrow_positions: u32,
}

/// Minimum borrow collateral changed, in USD (WAD).
#[contractevent(topics = ["config", "min_borrow_collateral"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateMinBorrowCollateralEvent {
    pub min_borrow_collateral_usd_wad: i128,
}

/// Hub created.
#[contractevent(topics = ["config", "hub"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateHubEvent {
    pub hub_id: u32,
}
