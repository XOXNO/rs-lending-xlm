pub use common::constants::*;

/// Collateral value at or below this USD WAD threshold may be socialized.
pub const BAD_DEBT_USD_THRESHOLD: i128 = 5 * WAD;

// Public view helpers accept caller-provided vectors. Bound them so malformed
// RPC reads cannot force unbounded local work.
pub const MAX_VIEW_INPUTS: u32 = 256;

/// Default liquidation curve target health factor (USD WAD, 1.02). Used when a
/// spoke leaves `liquidation_target_hf_wad` at zero.
pub const DEFAULT_LIQUIDATION_TARGET_HF_WAD: i128 = 1_020_000_000_000_000_000;

/// Default liquidation bonus factor (BPS, 1.0x). At this value the bonus
/// increment is unscaled, preserving the legacy curve exactly.
pub const DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS: u32 = 10_000;
