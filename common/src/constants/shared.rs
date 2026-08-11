//! Protocol-wide scaling factors, decimal bounds, price/tolerance/liquidation
//! limits, and storage TTL thresholds/bump amounts shared across contracts.

/// Ray fixed-point scale: one unit at 27 decimal places.
pub const RAY: i128 = 1_000_000_000_000_000_000_000_000_000;

/// Wad fixed-point scale: one unit at 18 decimal places.
pub const WAD: i128 = 1_000_000_000_000_000_000;

/// Basis-points denominator: 10,000 basis points equal 100%.
pub const BPS: i128 = 10_000;

/// Number of decimal places represented by [`RAY`].
pub const RAY_DECIMALS: u32 = 27;

/// Number of decimal places represented by [`WAD`].
pub const WAD_DECIMALS: u32 = 18;

/// Minimum allowed market / listed-token decimals (governance + price-aggregator).
pub const MIN_ASSET_DECIMALS: u32 = 3;

/// Maximum allowed market / listed-token decimals (matches WAD-scale prices).
pub const MAX_ASSET_DECIMALS: u32 = 18;

/// Approximate milliseconds per year (~365.2422 days), used to convert
/// annualized rates to a per-millisecond basis.
pub const MILLISECONDS_PER_YEAR: u64 = 31_556_926_000;

/// Upper bound accepted for an oracle price or sanity bound, in wad units.
pub const MAX_REASONABLE_PRICE_WAD: i128 = 1_000_000_000 * WAD;

/// Upper bound accepted for a liquidation curve's target health factor, in wad units.
pub const MAX_LIQUIDATION_TARGET_HF_WAD: i128 = 10 * WAD;

/// Default minimum USD collateral value required to open a borrow position, in wad units.
pub const DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD: i128 = 5 * WAD;

/// Upper bound accepted for a pool's configured flash-loan fee, in basis points.
pub const MAX_FLASHLOAN_FEE_BPS: i128 = 500;

/// Upper bound accepted for the maximum number of supply or borrow positions an account may hold.
pub const POSITION_LIMIT_MAX: u32 = 10;

/// Lower bound accepted for an oracle tolerance value, in basis points.
pub const MIN_TOLERANCE: u32 = 150;

/// Minimum relative half-width of an oracle sanity band, in basis points of
/// `(max_wad + min_wad)`: requires `(max - min) * BPS / (max + min) >= this`.
pub const MIN_SANITY_BAND_BPS: i128 = 50;

/// Upper bound accepted for an oracle tolerance value, in basis points.
pub const MAX_TOLERANCE: u32 = 2_500;

/// Number of milliseconds in one second.
pub const MS_PER_SECOND: u64 = 1_000;

/// Approximate number of ledgers produced in one day, used to derive the TTL
/// thresholds and bump amounts below.
pub(crate) const ONE_DAY_LEDGERS: u32 = 17_280;

const TTL_THRESHOLD_USER_DAYS: u32 = 30;

const TTL_THRESHOLD_SAFETY_DAYS: u32 = 5;
const TTL_BUMP_INSTANCE_DAYS: u32 = 180;
const TTL_BUMP_SHARED_DAYS: u32 = 180;
const TTL_BUMP_USER_DAYS: u32 = 120;

/// Live-until-ledger threshold below which instance storage TTL extension is triggered.
pub const TTL_THRESHOLD_INSTANCE: u32 = ONE_DAY_LEDGERS * TTL_THRESHOLD_SAFETY_DAYS;
/// Number of ledgers instance storage TTL is extended to when renewed.
pub const TTL_BUMP_INSTANCE: u32 = ONE_DAY_LEDGERS * TTL_BUMP_INSTANCE_DAYS;

/// Live-until-ledger threshold below which shared persistent storage TTL extension is triggered.
pub const TTL_THRESHOLD_SHARED: u32 = ONE_DAY_LEDGERS * TTL_THRESHOLD_SAFETY_DAYS;
/// Number of ledgers shared persistent storage TTL is extended to when renewed.
pub const TTL_BUMP_SHARED: u32 = ONE_DAY_LEDGERS * TTL_BUMP_SHARED_DAYS;

/// Live-until-ledger threshold below which per-user persistent storage TTL extension is triggered.
pub const TTL_THRESHOLD_USER: u32 = ONE_DAY_LEDGERS * TTL_THRESHOLD_USER_DAYS;
/// Number of ledgers per-user persistent storage TTL is extended to when renewed.
pub const TTL_BUMP_USER: u32 = ONE_DAY_LEDGERS * TTL_BUMP_USER_DAYS;
