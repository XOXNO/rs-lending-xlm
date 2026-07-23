//! Pool supply-index band and borrow rate/index ceilings.

use crate::constants::RAY;

/// Minimum supply index after bad-debt socialization. At `RAY / 1000`, a full
/// wipeout still leaves `calculate_scaled_supply` inside `i128` (~1.7e8 asset
/// units per deposit).
pub const SUPPLY_INDEX_FLOOR_RAW: i128 = RAY / 1_000;

/// Maximum annual borrow rate accepted by the interest model.
pub const MAX_BORROW_RATE_RAY: i128 = 2 * RAY;

/// Ceiling on `borrow_index`; bounds compounding before i128-backed values overflow.
pub const MAX_BORROW_INDEX_RAY: i128 = 1_000_000_000_000_000_000_000_000_000_000_000_000;

/// Ceiling on `supply_index`; mirrors `MAX_BORROW_INDEX_RAY` so reward-driven
/// growth cannot leave the range where share conversions stay inside `i128`.
pub const MAX_SUPPLY_INDEX_RAY: i128 = MAX_BORROW_INDEX_RAY;

/// Caps reward-driven `supply_index` growth (`add_rewards` only) so repeated
/// reward legs cannot pin the index at `MAX_SUPPLY_INDEX_RAY`.
pub const SUPPLY_INDEX_REWARD_CEILING_RAY: i128 = 100_000 * RAY;

/// Phantom supply value added to the reward denominator in `update_supply_index`,
/// so a dust supplier can neither inflate the index nor recover the reward.
/// Value-scaled (1 token = `RAY`), decimal-independent.
pub const SUPPLY_VIRTUAL_VALUE_RAY: i128 = RAY;
