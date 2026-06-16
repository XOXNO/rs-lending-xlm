//! Governance constants.

/// Mainnet minimum timelock delay in ledgers: 48h at ~5s per ledger.
/// Constructor parameters may use shorter delays on non-mainnet deployments.
pub const TIMELOCK_MIN_DELAY_LEDGERS: u32 = 34_560;

/// Expiration window for Ready operations, in ledgers.
/// Prevents stale Ready operations from remaining executable indefinitely.
pub const TIMELOCK_OPERATION_GRACE_LEDGERS: u32 = 120_960;
