pub use common::constants::*;

/// Collateral value at or below this USD WAD threshold may be socialized.
pub const BAD_DEBT_USD_THRESHOLD: i128 = 5 * WAD;

// Cap on the per-category assets map; bounded so the serialized category
// fits comfortably under the Soroban per-entry size limit (~65 KiB).
pub const MAX_EMODE_ASSETS_PER_CATEGORY: u32 = 64;

// Cap on the controller's `PoolsList`. Single-entry serialization + per-tx
// footprint cost both scale linearly with the list length.
pub const MAX_POOLS_LIST_ENTRIES: u32 = 256;
