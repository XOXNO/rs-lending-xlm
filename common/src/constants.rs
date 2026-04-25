pub const RAY: i128 = 1_000_000_000_000_000_000_000_000_000;

pub const WAD: i128 = 1_000_000_000_000_000_000;

pub const BPS: i128 = 10_000;

pub const RAY_DECIMALS: u32 = 27;

pub const WAD_DECIMALS: u32 = 18;

pub const BPS_DECIMALS: u32 = 4;

pub const MILLISECONDS_PER_YEAR: u64 = 31_556_926_000;

pub const MAX_LIQUIDATION_BONUS: i128 = 1_500;

/// Lower clamp on the post-bad-debt supply index, in raw RAY units. The pool
/// floors `supply_index_ray` at this value during bad-debt socialization
/// (`pool/src/interest.rs::apply_bad_debt_to_supply_index`). Keeping the
/// constant here lets Certora rules reference the single source of truth and
/// avoids silent drift if the pool-side constant changes.
pub const SUPPLY_INDEX_FLOOR_RAW: i128 = WAD;

/// Bad-debt socialization threshold: an account with collateral at or
/// below $5 USD AND debt > collateral triggers `apply_bad_debt_to_supply_index`.
/// Stored in WAD precision (1 USD = 10^18). Referenced by liquidation paths
/// and `clean_bad_debt_standalone`.
pub const BAD_DEBT_USD_THRESHOLD: i128 = 5 * WAD;

/// Maximum permitted flash-loan fee, in BPS. 500 = 5%. Validated at both
/// `create_liquidity_pool` (via `validate_asset_config`) and `edit_asset_config`.
pub const MAX_FLASHLOAN_FEE_BPS: i128 = 500;

/// Upper cap on `max_borrow_rate_ray`. The compound-interest 8-term Taylor
/// series in `pool/src/interest.rs` has documented `< 0.01 %` accuracy only
/// for per-chunk `x = rate * delta_time / RAY <= 2 RAY`. Capping
/// `max_borrow_rate_ray` at `2 * RAY` keeps interest accrual inside the
/// proven envelope even at 100 % utilization across a full
/// `MAX_COMPOUND_DELTA_MS` chunk. Validated by both
/// `controller/src/validation::validate_interest_rate_model` and
/// `pool::Pool::update_params`.
pub const MAX_BORROW_RATE_RAY: i128 = 2 * RAY;

pub const K_SCALING_FACTOR: i128 = 20_000;

pub const MIN_FIRST_TOLERANCE: i128 = 50;

pub const MAX_FIRST_TOLERANCE: i128 = 5_000;

pub const MIN_LAST_TOLERANCE: i128 = 150;

/// Absolute ceiling on oracle last-tolerance (BPS). Enforced in
/// `validate_oracle_bounds`.
pub const MAX_LAST_TOLERANCE: i128 = 5_000;

pub const ONE_DAY_LEDGERS: u32 = 17_280;

// ---------------------------------------------------------------------------
// Tiered storage TTLs (ledger counts)
// ---------------------------------------------------------------------------

pub const TTL_THRESHOLD_INSTANCE: u32 = ONE_DAY_LEDGERS * 120; // ~120 days
pub const TTL_BUMP_INSTANCE: u32 = ONE_DAY_LEDGERS * 180; // ~180 days (Soroban max)

pub const TTL_THRESHOLD_SHARED: u32 = ONE_DAY_LEDGERS * 30; // ~30 days
pub const TTL_BUMP_SHARED: u32 = ONE_DAY_LEDGERS * 120; // ~120 days

pub const TTL_THRESHOLD_USER: u32 = ONE_DAY_LEDGERS * 100; // ~100 days
pub const TTL_BUMP_USER: u32 = ONE_DAY_LEDGERS * 120; // ~120 days

pub const TTL_THRESHOLD: u32 = TTL_THRESHOLD_INSTANCE;
pub const TTL_EXTEND_TO: u32 = TTL_BUMP_INSTANCE;

pub const MAX_SUPPLY_POSITIONS: u8 = 4;

pub const MAX_BORROW_POSITIONS: u8 = 4;
