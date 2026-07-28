//! Timelock delay floors, caps, and execution-grace windows (ledger units).
//!
//! Durations assume ~5s per ledger. Constructor `min_delay` may be lower than
//! [`TIMELOCK_MIN_DELAY_LEDGERS`] on non-mainnet deployments; sensitive and
//! recovery floors still apply per operation.

/// Standard-tier mainnet floor: 48h.
pub const TIMELOCK_MIN_DELAY_LEDGERS: u32 = 34_560;

/// Maximum delay accepted by `AdminOperation::UpdateGovDelay`: 14 days.
pub const TIMELOCK_MAX_DELAY_LEDGERS: u32 = 241_920;

/// Sensitive-tier floor. Applied to wasm upgrades, ownership transfers,
/// price-aggregator re-point, role grant/revoke, and force bad-debt socialization,
/// even when `get_min_delay` is lower.
///
/// TEMPORARY pre-audit value: 12 ledgers (~1 min) so fixes ship without a
/// 7-day wait during the audit window. The effective sensitive delay is
/// `max(get_min_delay, this)`, so the configured `min_delay` still applies.
/// Once audits conclude, upgrade this contract restoring the production
/// floor of 7 days: 120_960.
pub const TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS: u32 = 12;

/// How long a Ready operation remains executable after its ready ledger: 7 days.
pub const TIMELOCK_OPERATION_GRACE_LEDGERS: u32 = 120_960;

/// Recovery-tier floor: ~30 days. Canceller-council reset only; non-vetoable.
pub const TIMELOCK_RECOVERY_MIN_DELAY_LEDGERS: u32 = 518_400;
