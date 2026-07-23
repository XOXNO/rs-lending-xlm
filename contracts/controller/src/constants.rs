//! Protocol bounds and defaults owned by the controller.

pub use common::constants::*;

/// Collateral value at or below this USD WAD threshold may be socialized.
pub const BAD_DEBT_USD_THRESHOLD: i128 = 5 * WAD;

// Bound view input vectors (malformed RPC cost).
pub const MAX_VIEW_INPUTS: u32 = 256;

/// Minimum HF (1.05 WAD) required before lowering a position's liquidation threshold.
pub const THRESHOLD_UPDATE_MIN_HF_RAW: i128 = 1_050_000_000_000_000_000;

/// Default liquidation target HF (USD WAD, 1.10). Stamped into `SpokeConfig` at create.
pub const DEFAULT_LIQUIDATION_TARGET_HF_WAD: i128 = 1_100_000_000_000_000_000;

/// Default HF for max bonus (WAD 0.80): full bonus once collateral no longer covers its debt.
pub const DEFAULT_HF_FOR_MAX_BONUS_WAD: i128 = 800_000_000_000_000_000;

/// Default liquidation bonus factor (BPS, 1.0x unscaled). Stamped at spoke create.
pub const DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS: u32 = 10_000;

/// Pool ABI sentinel for full-position withdraw (user amount `0` maps here).
pub const WITHDRAW_ALL_SENTINEL: i128 = i128::MAX;

/// Cap on per-account delegates. The list loads as one persistent entry, so it
/// stays bounded; mirrors the instance-tier approval caps.
pub const MAX_DELEGATES: u32 = 16;
