//! Controller-specific constants. Re-exports the shared constants from
//! `common::constants` and defines the numeric limits and default risk-parameter
//! values used across the controller contract.

pub use common::constants::*;

/// Socializable bad-debt ceiling, denominated in USD WAD. Equal to
/// [`DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD`].
pub const BAD_DEBT_USD_THRESHOLD: i128 = DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD;

/// Maximum number of elements accepted in a single batched view-function input vector.
pub const MAX_VIEW_INPUTS: u32 = 256;

/// Minimum account health factor, as a raw WAD value, required after a risk-parameter
/// update that could tighten an account's position.
pub const THRESHOLD_UPDATE_MIN_HF_RAW: i128 = 1_050_000_000_000_000_000;

/// Default health factor, in WAD, that a liquidation restores an account to. Assigned
/// to a spoke's `liquidation_target_hf_wad` when the spoke is added.
pub const DEFAULT_LIQUIDATION_TARGET_HF_WAD: i128 = 1_100_000_000_000_000_000;

/// Default health factor, in WAD, at or below which the liquidation bonus reaches its
/// maximum. Assigned to a spoke's `hf_for_max_bonus_wad` when the spoke is added.
pub const DEFAULT_HF_FOR_MAX_BONUS_WAD: i128 = 800_000_000_000_000_000;

/// Default liquidation bonus factor: `1.0` in BPS (`BPS` / 100%).
pub const DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS: u32 = BPS as u32;

/// Sentinel withdrawal amount that requests withdrawal of the entire available balance.
pub const WITHDRAW_ALL_SENTINEL: i128 = i128::MAX;

/// Maximum number of delegates an account may register.
pub const MAX_DELEGATES: u32 = 16;

/// App version stored on first initialization, before any migration runs.
pub const INITIAL_APP_VERSION: u32 = 1;
