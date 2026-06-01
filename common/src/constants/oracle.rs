// Protocol-wide upper bound for operator-supplied per-asset sanity caps.
pub const MAX_REASONABLE_PRICE_WAD: i128 = 1_000_000_000 * super::WAD;

pub const MIN_FIRST_TOLERANCE: i128 = 50;

pub const MAX_FIRST_TOLERANCE: i128 = 5_000;

pub const MIN_LAST_TOLERANCE: i128 = 150;

/// Maximum last-price tolerance in BPS.
pub const MAX_LAST_TOLERANCE: i128 = 5_000;
