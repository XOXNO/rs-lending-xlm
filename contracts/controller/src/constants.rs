//! Bad-debt threshold, view-input bound, and default spoke liquidation curve.

pub use common::constants::*;

/// Collateral value at or below this USD WAD threshold may be socialized.
pub const BAD_DEBT_USD_THRESHOLD: i128 = 5 * WAD;

// Bound view input vectors (malformed RPC cost).
pub const MAX_VIEW_INPUTS: u32 = 256;

/// Default liquidation target HF (USD WAD, 1.10). Stamped into `SpokeConfig` at create.
pub const DEFAULT_LIQUIDATION_TARGET_HF_WAD: i128 = 1_100_000_000_000_000_000;

/// Default HF for max bonus (WAD 0.80): full bonus once collateral no longer covers its debt.
pub const DEFAULT_HF_FOR_MAX_BONUS_WAD: i128 = 800_000_000_000_000_000;

/// Default liquidation bonus factor (BPS, 1.0x unscaled). Stamped at spoke create.
pub const DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS: u32 = 10_000;
