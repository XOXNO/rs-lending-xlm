pub use common::constants::*;

pub const BAD_DEBT_USD_THRESHOLD: i128 = DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD;

pub const MAX_VIEW_INPUTS: u32 = 256;

pub const THRESHOLD_UPDATE_MIN_HF_RAW: i128 = 1_050_000_000_000_000_000;

pub const DEFAULT_LIQUIDATION_TARGET_HF_WAD: i128 = 1_100_000_000_000_000_000;

pub const DEFAULT_HF_FOR_MAX_BONUS_WAD: i128 = 800_000_000_000_000_000;

pub const DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS: u32 = BPS as u32;

pub const WITHDRAW_ALL_SENTINEL: i128 = i128::MAX;

/// Withdraw-all amount the Blend migration sends. Blend converts a withdraw
/// request to b-tokens, `ceil(amount * 1e12 / b_rate)`, before clamping it to
/// the position, and traps on overflow, so `i128::MAX` traps once bad debt
/// pushes a reserve's b_rate below 1.0. 1e30 converts without overflow while
/// b_rate stays above 5.9e-9, and exceeds any position up to 1e12 whole tokens
/// at `MAX_ASSET_DECIMALS`. A larger position is swept only in part; the rest
/// stays in Blend.
pub const BLEND_WITHDRAW_ALL_AMOUNT: i128 = 1_000_000_000_000_000_000_000_000_000_000;

pub const MAX_DELEGATES: u32 = 16;

pub const INITIAL_APP_VERSION: u32 = 1;
