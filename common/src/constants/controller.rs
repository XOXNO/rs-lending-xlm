/// Collateral value at or below this USD WAD threshold may be socialized.
pub const BAD_DEBT_USD_THRESHOLD: i128 = 5 * super::WAD;

/// Minimum non-zero dust floor accepted in market config, denominated in USD WAD.
pub const MIN_DUST_FLOOR_WAD: i128 = 10 * super::WAD;

/// Maximum flash-loan and strategy fee in BPS.
pub const MAX_FLASHLOAN_FEE_BPS: i128 = 500;

pub const MAX_SUPPLY_POSITIONS: u8 = 4;

pub const MAX_BORROW_POSITIONS: u8 = 4;

// Cap on the per-category assets map; bounded so the serialized category
// fits comfortably under the Soroban per-entry size limit (~65 KiB).
pub const MAX_EMODE_ASSETS_PER_CATEGORY: u32 = 64;

// Cap on the controller's `PoolsList`. Single-entry serialization + per-tx
// footprint cost both scale linearly with the list length.
pub const MAX_POOLS_LIST_ENTRIES: u32 = 256;
