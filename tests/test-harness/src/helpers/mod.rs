pub mod units;

pub use crate::oracle::config::*;
pub use units::*;

use common::types::HubAssetKey;
use soroban_sdk::Address;

/// The single hub the base setup creates and lists every market under. There is
/// no hub 0 anymore: a fresh controller has zero hubs, so the builder calls
/// `create_hub()` (which returns id 1) before listing markets. Multi-hub tests
/// create additional hubs (ids 2+) on top of this one.
pub const HARNESS_HUB: u32 = 1;

/// The single spoke the base setup creates and lists every market on with its
/// regular (non-spoke) risk params. There is no spoke 0 anymore: a fresh
/// controller has zero spokes, so the builder calls `add_spoke()` (which returns
/// id 1) before listing market risk. Regular accounts bind to this spoke;
/// spoke tests create additional spokes (ids 2+) on top of it.
pub const HARNESS_SPOKE: u32 = 1;

/// Wraps an asset address in the base harness hub coordinate used by controller
/// endpoints.
pub fn hub_asset(asset: Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: HARNESS_HUB,
        asset,
    }
}
