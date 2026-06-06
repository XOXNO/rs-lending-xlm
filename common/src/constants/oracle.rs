// Protocol-wide upper bound for operator-supplied per-asset sanity caps.
pub const MAX_REASONABLE_PRICE_WAD: i128 = 1_000_000_000 * super::WAD;

/// Minimum first-price tolerance input (BPS) for `configure_market_oracle`.
pub const MIN_FIRST_TOLERANCE: u32 = 50;

/// Maximum first-price tolerance input (BPS).
pub const MAX_FIRST_TOLERANCE: u32 = 5_000;

/// Minimum last-price tolerance input (BPS).
pub const MIN_LAST_TOLERANCE: u32 = 150;

/// Maximum last-price tolerance input (BPS).
pub const MAX_LAST_TOLERANCE: u32 = 5_000;
