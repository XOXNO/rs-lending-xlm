pub use common::constants::*;

/// Collateral value at or below this USD WAD threshold may be socialized.
pub const BAD_DEBT_USD_THRESHOLD: i128 = 5 * WAD;

// Public view helpers accept caller-provided vectors. Bound them so malformed
// RPC reads cannot force unbounded local work.
pub const MAX_VIEW_INPUTS: u32 = 256;
