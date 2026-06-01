pub const RAY: i128 = 1_000_000_000_000_000_000_000_000_000;

pub const WAD: i128 = 1_000_000_000_000_000_000;

pub const BPS: i128 = 10_000;

pub const RAY_DECIMALS: u32 = 27;

pub const WAD_DECIMALS: u32 = 18;

pub const BPS_DECIMALS: u32 = 4;

pub const MILLISECONDS_PER_YEAR: u64 = 31_556_926_000;

pub const MS_PER_SECOND: u64 = 1_000;

pub const ONE_DAY_LEDGERS: u32 = 17_280;

pub const TTL_THRESHOLD_INSTANCE: u32 = ONE_DAY_LEDGERS * 30; // ~30 days
pub const TTL_BUMP_INSTANCE: u32 = ONE_DAY_LEDGERS * 180; // ~180 days (Soroban max)

pub const TTL_THRESHOLD_SHARED: u32 = ONE_DAY_LEDGERS * 30; // ~30 days
pub const TTL_BUMP_SHARED: u32 = ONE_DAY_LEDGERS * 180; // ~180 days

pub const TTL_THRESHOLD_USER: u32 = ONE_DAY_LEDGERS * 30; // ~30 days
pub const TTL_BUMP_USER: u32 = ONE_DAY_LEDGERS * 120; // ~120 days

pub const TTL_THRESHOLD: u32 = TTL_THRESHOLD_INSTANCE;
pub const TTL_EXTEND_TO: u32 = TTL_BUMP_INSTANCE;
