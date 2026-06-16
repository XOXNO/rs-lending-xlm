pub use common::constants::*;

/// Collateral value at or below this USD WAD threshold may be socialized.
pub const BAD_DEBT_USD_THRESHOLD: i128 = 5 * WAD;

// Cap on the per-category assets map; bounded so the serialized category
// fits comfortably under the Soroban per-entry size limit (~65 KiB).
pub const MAX_EMODE_ASSETS_PER_CATEGORY: u32 = 64;

// Cap on the controller's `PoolsList`. Single-entry serialization + per-tx
// footprint cost both scale linearly with the list length.
pub const MAX_POOLS_LIST_ENTRIES: u32 = 256;

// Public view helpers accept caller-provided vectors. Bound them to the same
// order of magnitude as the market registry so malformed RPC reads cannot force
// unbounded local work.
pub const MAX_VIEW_INPUTS: u32 = MAX_POOLS_LIST_ENTRIES;
