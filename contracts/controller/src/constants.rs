//! Controller-level constants: the bad-debt socialization threshold, the
//! view-input bound, and the default liquidation-curve parameters stamped into
//! each spoke at creation.

pub use common::constants::*;

/// Collateral value at or below this USD WAD threshold may be socialized.
pub const BAD_DEBT_USD_THRESHOLD: i128 = 5 * WAD;

// Public view helpers accept caller-provided vectors. Bound them so malformed
// RPC reads cannot force unbounded local work.
pub const MAX_VIEW_INPUTS: u32 = 256;

/// Default liquidation curve target health factor (USD WAD, 1.10). Stamped
/// into `SpokeConfig` at spoke creation; liquidation reads storage verbatim.
/// Restoring to 10% above the threshold keeps volatile collateral from
/// re-tripping liquidation on small moves; low-volatility spokes (e.g. a
/// stables-only spoke) override downward via `set_spoke_liquidation_curve`.
pub const DEFAULT_LIQUIDATION_TARGET_HF_WAD: i128 = 1_100_000_000_000_000_000;

/// Default HF at which the liquidation bonus reaches its maximum (USD WAD,
/// 0.80). Sits just above the highest effective liquidation threshold in
/// use, so by the time an account's collateral no longer covers its debt
/// (HF < effective threshold) liquidators already earn the full bonus.
/// Stamped into `SpokeConfig` at spoke creation.
pub const DEFAULT_HF_FOR_MAX_BONUS_WAD: i128 = 800_000_000_000_000_000;

/// Default liquidation bonus factor (BPS, 1.0x). At this value the bonus
/// increment is unscaled. Stamped into `SpokeConfig` at spoke creation.
pub const DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS: u32 = 10_000;
