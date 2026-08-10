pub use common::constants::*;

/// Socializable bad-debt ceiling (USD WAD). Same magnitude as
/// [`DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD`]; kept as a named policy knob so
/// either can move independently without a silent coupling.
pub const BAD_DEBT_USD_THRESHOLD: i128 = DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD;

pub const MAX_VIEW_INPUTS: u32 = 256;

pub const THRESHOLD_UPDATE_MIN_HF_RAW: i128 = 1_050_000_000_000_000_000;

pub const DEFAULT_LIQUIDATION_TARGET_HF_WAD: i128 = 1_100_000_000_000_000_000;

pub const DEFAULT_HF_FOR_MAX_BONUS_WAD: i128 = 800_000_000_000_000_000;

/// Default liquidation bonus factor: `1.0` in BPS (`BPS` / 100%).
pub const DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS: u32 = BPS as u32;

pub const WITHDRAW_ALL_SENTINEL: i128 = i128::MAX;

pub const MAX_DELEGATES: u32 = 16;

pub const INITIAL_APP_VERSION: u32 = 1;
