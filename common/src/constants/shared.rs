pub const RAY: i128 = 1_000_000_000_000_000_000_000_000_000;

pub const WAD: i128 = 1_000_000_000_000_000_000;

pub const BPS: i128 = 10_000;

pub const RAY_DECIMALS: u32 = 27;

pub const WAD_DECIMALS: u32 = 18;

pub const MILLISECONDS_PER_YEAR: u64 = 31_556_926_000;

// Oracle-config price bounds.

// Protocol-wide upper bound for operator-supplied per-asset price caps.
pub const MAX_REASONABLE_PRICE_WAD: i128 = 1_000_000_000 * WAD;

/// Default instance-level minimum LTV-weighted collateral (USD WAD) required
/// while an account carries debt.
pub const DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD: i128 = 5 * WAD;

/// Maximum flash-loan and strategy fee in BPS.
pub const MAX_FLASHLOAN_FEE_BPS: i128 = 500;

/// Maximum supply/borrow positions configurable per account (protocol-wide).
pub const POSITION_LIMIT_MAX: u32 = 10;

/// Minimum first-price tolerance input (BPS) for oracle config validation.
pub const MIN_FIRST_TOLERANCE: u32 = 50;

/// Maximum first-price tolerance input (BPS).
pub const MAX_FIRST_TOLERANCE: u32 = 500;

/// Minimum last-price tolerance input (BPS).
pub const MIN_LAST_TOLERANCE: u32 = 150;

/// Maximum last-price tolerance input (BPS).
pub const MAX_LAST_TOLERANCE: u32 = 5_000;

pub const MS_PER_SECOND: u64 = 1_000;

pub(crate) const ONE_DAY_LEDGERS: u32 = 17_280;

const TTL_THRESHOLD_DAYS: u32 = 30;
const TTL_BUMP_INSTANCE_DAYS: u32 = 180;
const TTL_BUMP_SHARED_DAYS: u32 = 180;
const TTL_BUMP_USER_DAYS: u32 = 120;

pub const TTL_THRESHOLD_INSTANCE: u32 = ONE_DAY_LEDGERS * TTL_THRESHOLD_DAYS;
pub const TTL_BUMP_INSTANCE: u32 = ONE_DAY_LEDGERS * TTL_BUMP_INSTANCE_DAYS;

pub const TTL_THRESHOLD_SHARED: u32 = ONE_DAY_LEDGERS * TTL_THRESHOLD_DAYS;
pub const TTL_BUMP_SHARED: u32 = ONE_DAY_LEDGERS * TTL_BUMP_SHARED_DAYS;

pub const TTL_THRESHOLD_USER: u32 = ONE_DAY_LEDGERS * TTL_THRESHOLD_DAYS;
pub const TTL_BUMP_USER: u32 = ONE_DAY_LEDGERS * TTL_BUMP_USER_DAYS;
