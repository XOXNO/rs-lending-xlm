//! Defines the contract events emitted for controller configuration changes:
//! spoke and spoke-asset registration, blend-pool approval, aggregator and
//! position-limit updates, and hub creation.

use soroban_sdk::{contractevent, contracttype, Address};

use common::types::{SpokeAssetConfig, SpokeConfig};

/// Snapshot of a spoke's liquidation-relevant configuration fields, embedded
/// in [`UpdateSpokeEvent`].
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
    /// Builds an `EventSpoke` snapshot from `spoke_id` and the deprecation,
    /// liquidation-target, max-bonus, and bonus-factor fields of `spoke`.
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

/// Event recording that a spoke's configuration is created or updated.
#[contractevent(topics = ["config", "spoke"])]
#[derive(Clone, Debug)]
pub struct UpdateSpokeEvent {
    pub spoke: EventSpoke,
}

/// Event recording that an asset's configuration within a spoke is created or
/// updated.
#[contractevent(topics = ["config", "spoke_asset"])]
#[derive(Clone, Debug)]
pub struct UpdateSpokeAssetEvent {
    pub asset: Address,
    pub config: SpokeAssetConfig,
    pub spoke_id: u32,
    pub hub_id: u32,
}

/// Event recording that an asset's configuration is removed from a spoke.
#[contractevent(topics = ["config", "remove_spoke_asset"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoveSpokeAssetEvent {
    pub asset: Address,
    pub spoke_id: u32,
    pub hub_id: u32,
}

/// Event recording that a Blend pool's approval status changes.
#[contractevent(topics = ["config", "approve_blend_pool"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApproveBlendPoolEvent {
    pub pool: Address,
    pub approved: bool,
}

/// Event recording that the configured swap aggregator address changes.
#[contractevent(topics = ["config", "swap_aggregator"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateSwapAggregatorEvent {
    pub swap_aggregator: Address,
}

/// Event recording that the configured price aggregator address changes.
#[contractevent(topics = ["config", "price_aggregator"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdatePriceAggregatorEvent {
    pub price_aggregator: Address,
}

/// Event recording that the configured accumulator address changes.
#[contractevent(topics = ["config", "accumulator"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateAccumulatorEvent {
    pub accumulator: Address,
}

/// Event recording that the protocol-wide maximum supply and borrow position
/// counts change.
#[contractevent(topics = ["config", "position_limits"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdatePositionLimitsEvent {
    pub max_supply_positions: u32,
    pub max_borrow_positions: u32,
}

/// Event recording that the minimum borrow collateral value (in USD, WAD
/// scale) changes.
#[contractevent(topics = ["config", "min_borrow_collateral"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateMinBorrowCollateralEvent {
    pub min_borrow_collateral_usd_wad: i128,
}

/// Event recording that a new hub is created.
#[contractevent(topics = ["config", "hub"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateHubEvent {
    pub hub_id: u32,
}
