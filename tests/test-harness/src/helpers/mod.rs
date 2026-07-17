pub mod units;

pub use crate::oracle::config::*;
pub use units::*;

use common::types::HubAssetKey;
use soroban_sdk::Address;

/// Base harness hub id is 1 (`create_hub()` on empty controller). Multi-hub tests add hubs 2+.
pub const HARNESS_HUB: u32 = 1;

/// Base harness spoke id is 1. Regular accounts bind here; spoke tests add spokes 2+.
pub const HARNESS_SPOKE: u32 = 1;

/// Wraps an asset address in the base harness hub coordinate used by controller
/// endpoints.
pub fn hub_asset(asset: Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: HARNESS_HUB,
        asset,
    }
}
