pub(crate) mod units;

pub use crate::oracle::config::*;
pub use units::*;

use common::types::HubAssetKey;
use soroban_sdk::Address;

pub const HARNESS_HUB: u32 = 1;

pub const HARNESS_SPOKE: u32 = 1;

pub fn hub_asset(asset: Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: HARNESS_HUB,
        asset,
    }
}
