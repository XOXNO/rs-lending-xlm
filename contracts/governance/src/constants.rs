//! Timelock delay bounds and operation-grace constants (ledger units).

/// Mainnet minimum timelock delay in ledgers: 48h at ~5s per ledger.
/// Constructor parameters may use shorter delays on non-mainnet deployments.
pub const TIMELOCK_MIN_DELAY_LEDGERS: u32 = 34_560;

/// Upper bound accepted by `AdminOperation::UpdateGovDelay`: 14 days at ~5s per ledger.
pub const TIMELOCK_MAX_DELAY_LEDGERS: u32 = 241_920;

/// Minimum schedule delay for Sensitive-tier proposals — wasm upgrades
/// (governance, controller, pool), ownership transfers (governance, controller),
/// and re-pointing the controller's price-aggregator (oracle authority): 7 days
/// at ~5s per ledger. Applied per operation even when `get_min_delay` is lower.
pub const TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS: u32 = 120_960;

/// Expiration window for Ready operations, in ledgers.
/// Prevents stale Ready operations from remaining executable indefinitely.
pub const TIMELOCK_OPERATION_GRACE_LEDGERS: u32 = 120_960;

/// Minimum delay for the Recovery tier — non-vetoable council reset: ~30 days
/// at ~5s per ledger. Long and public so it cannot serve as a quiet theft path
/// even for a compromised owner multisig.
pub const TIMELOCK_RECOVERY_MIN_DELAY_LEDGERS: u32 = 518_400;
