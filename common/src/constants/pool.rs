//! Bounds for the pool's supply/borrow interest indices and configured
//! borrow-rate ceiling, expressed in ray (1e27) fixed-point scale.

use crate::constants::RAY;

/// Minimum value the supply index is clamped to after accrual or write-down, in raw ray units.
pub const SUPPLY_INDEX_FLOOR_RAW: i128 = RAY / 1_000;

/// Upper bound accepted for a pool's configured maximum borrow rate, in raw ray units.
pub const MAX_BORROW_RATE_RAY: i128 = 2 * RAY;

/// Ceiling the borrow index is clamped to after growth, in raw ray units.
pub const MAX_BORROW_INDEX_RAY: i128 = 1_000_000_000_000_000_000_000_000_000_000_000_000;

/// Ceiling the supply index is clamped to after growth, in raw ray units.
/// Equal to [`MAX_BORROW_INDEX_RAY`].
pub const MAX_SUPPLY_INDEX_RAY: i128 = MAX_BORROW_INDEX_RAY;
