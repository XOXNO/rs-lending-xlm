//! Timelock delay and grace-period constants that size the governance
//! contract's operation tiers.

/// Suggested constructor `min_delay`, in ledgers (about 2 days). The contract
/// does not enforce it.
pub const TIMELOCK_MIN_DELAY_LEDGERS: u32 = 34_560;

/// Upper bound, in ledgers, allowed when updating the timelock's minimum
/// delay through a governance-executed delay-update operation.
pub const TIMELOCK_MAX_DELAY_LEDGERS: u32 = 241_920;

// Audit-period value. The target value is 120_960 (about 7 days), set through
// a governance-executed `UpgradeGov`.
/// Floor, in ledgers, applied to the delay for sensitive-tier operations:
/// the configured minimum delay is raised to at least this value for that
/// tier.
pub const TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS: u32 = 12;

/// Number of ledgers past an operation's ready ledger during which it
/// remains executable before it is treated as expired.
pub const TIMELOCK_OPERATION_GRACE_LEDGERS: u32 = 120_960;

/// Floor, in ledgers, applied to the delay for recovery-tier operations:
/// the configured minimum delay is raised to at least this value for that
/// tier.
pub const TIMELOCK_RECOVERY_MIN_DELAY_LEDGERS: u32 = 518_400;
