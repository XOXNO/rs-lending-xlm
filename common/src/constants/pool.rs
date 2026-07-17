//! Pool supply-index floor and borrow rate/index ceilings.

use crate::constants::{RAY, WAD};

/// Minimum supply index after bad-debt socialization.
pub const SUPPLY_INDEX_FLOOR_RAW: i128 = WAD;

/// Maximum annual borrow rate accepted by the interest model.
pub const MAX_BORROW_RATE_RAY: i128 = 2 * RAY;

/// Ceiling on `borrow_index`; bounds compounding before i128-backed values overflow.
pub const MAX_BORROW_INDEX_RAY: i128 = 1_000_000_000_000_000_000_000_000_000_000_000_000;
