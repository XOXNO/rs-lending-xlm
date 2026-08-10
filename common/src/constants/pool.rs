use crate::constants::RAY;

pub const SUPPLY_INDEX_FLOOR_RAW: i128 = RAY / 1_000;

pub const MAX_BORROW_RATE_RAY: i128 = 2 * RAY;

/// Share of supplied value that ordinary borrows may not draw below, reserved
/// so a seizure is not blocked by cash the borrowers took first.
pub const LIQUIDATION_BUFFER_BPS: i128 = 200;

pub const MAX_BORROW_INDEX_RAY: i128 = 1_000_000_000_000_000_000_000_000_000_000_000_000;

pub const MAX_SUPPLY_INDEX_RAY: i128 = MAX_BORROW_INDEX_RAY;
