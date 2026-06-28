pub mod units;

pub use crate::oracle::config::*;
pub use units::*;

use common::types::HubAssetKey;
use soroban_sdk::Address;

/// Wraps an asset address in the hub-0 coordinate used by controller endpoints.
pub fn hub_asset(asset: Address) -> HubAssetKey {
    HubAssetKey { hub_id: 0, asset }
}
