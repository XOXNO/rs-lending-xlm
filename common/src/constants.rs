pub const RAY: i128 = 1_000_000_000_000_000_000_000_000_000;

pub const WAD: i128 = 1_000_000_000_000_000_000;

pub const BPS: i128 = 10_000;

pub const RAY_DECIMALS: u32 = 27;

pub const WAD_DECIMALS: u32 = 18;

pub const BPS_DECIMALS: u32 = 4;

pub const MILLISECONDS_PER_YEAR: u64 = 31_556_926_000;

pub const MAX_LIQUIDATION_BONUS: i128 = 1_500;

pub const K_SCALING_FACTOR: i128 = 20_000;

pub const MIN_FIRST_TOLERANCE: i128 = 50;

pub const MAX_FIRST_TOLERANCE: i128 = 5_000;

pub const MIN_LAST_TOLERANCE: i128 = 150;

// 50% — hard safety ceiling. Matches `validate_oracle_bounds` in the controller.
pub const MAX_LAST_TOLERANCE: i128 = 5_000;

pub const ONE_DAY_LEDGERS: u32 = 17_280;

// ---------------------------------------------------------------------------
// Tiered TTL constants (matching Blend v2 best practices)
// ---------------------------------------------------------------------------

pub const TTL_THRESHOLD_INSTANCE: u32 = ONE_DAY_LEDGERS * 120; // ~120 days
pub const TTL_BUMP_INSTANCE: u32 = ONE_DAY_LEDGERS * 180; // ~180 days (Soroban max)

pub const TTL_THRESHOLD_SHARED: u32 = ONE_DAY_LEDGERS * 30; // ~30 days
pub const TTL_BUMP_SHARED: u32 = ONE_DAY_LEDGERS * 120; // ~120 days

pub const TTL_THRESHOLD_USER: u32 = ONE_DAY_LEDGERS * 100; // ~100 days
pub const TTL_BUMP_USER: u32 = ONE_DAY_LEDGERS * 120; // ~120 days

// Legacy aliases (for migration — will be removed)
pub const TTL_THRESHOLD: u32 = TTL_THRESHOLD_INSTANCE;
pub const TTL_EXTEND_TO: u32 = TTL_BUMP_INSTANCE;

pub const MAX_SUPPLY_POSITIONS: u8 = 4;

pub const MAX_BORROW_POSITIONS: u8 = 4;
