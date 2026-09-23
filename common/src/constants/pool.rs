//! Pool bounds: the supply index floor, the index ceilings and the borrow-rate
//! ceiling (RAY), and the liquidation cash buffer (BPS).

use crate::constants::RAY;

/// Minimum value the supply index is clamped to when bad debt is written down against
/// suppliers, in raw ray units. Interest accrual does not apply this floor; it only guarantees
/// the index never decreases.
pub const SUPPLY_INDEX_FLOOR_RAW: i128 = RAY / 1_000;

/// Upper bound accepted for a pool's configured maximum borrow rate, in raw ray units.
pub const MAX_BORROW_RATE_RAY: i128 = 2 * RAY;

/// Share of supplied value, in BPS, that pool cash must still cover after any
/// debt mint, borrows and strategy openings alike (INV-ACCT-07).
pub const LIQUIDATION_BUFFER_BPS: i128 = 200;

/// Ceiling the borrow index is clamped to after growth, in raw ray units.
pub const MAX_BORROW_INDEX_RAY: i128 = 1_000_000_000_000_000_000_000_000_000_000_000_000;

/// Ceiling the supply index is clamped to after growth, in raw ray units.
/// Equal to [`MAX_BORROW_INDEX_RAY`].
pub const MAX_SUPPLY_INDEX_RAY: i128 = MAX_BORROW_INDEX_RAY;
