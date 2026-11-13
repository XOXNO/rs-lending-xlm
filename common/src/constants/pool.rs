//! Pool bounds: the supply index floor, the index ceilings, the market value
//! and entry ceilings and the borrow-rate ceiling (RAY), and the liquidation
//! cash buffer (BPS).

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

/// Ceiling on a market's total supply value and on its total debt value
/// (`shares * index`, half up), in raw ray units: one whole token below
/// `i128::MAX`.
///
/// Accrual holds interest at this ceiling instead of overflowing
/// (INV-IDX-06), so both totals stay representable, also after an exit path
/// rounds one up to a whole token unit.
pub const MAX_MARKET_VALUE_RAY: i128 = i128::MAX - RAY;

/// Room left for interest between the entry ceiling and
/// [`MAX_MARKET_VALUE_RAY`], as a factor. One year at `MAX_BORROW_RATE_RAY`
/// multiplies debt by less than `e^2 < 8`.
pub const MARKET_ENTRY_HEADROOM: i128 = 8;

/// Ceiling that supply, borrow, strategy and flash-loan entries enforce on a
/// market's total supply and total debt, both as a share count and as a
/// value, in raw ray units (INV-ACCT-11). About 21.27 billion whole tokens.
///
/// The share count is bounded too because a supply index written down to
/// `SUPPLY_INDEX_FLOOR_RAW` mints 1,000 shares per unit of value.
pub const MAX_MARKET_ENTRY_RAY: i128 = MAX_MARKET_VALUE_RAY / MARKET_ENTRY_HEADROOM;
