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

/// Maximum number of `CANCELLER` holders, the owner included. A canceller
/// reset revokes every non-owner holder and grants the new list in one
/// transaction, so its cost grows with the council: replacing N non-owner
/// members with N new ones writes 3N + 6 ledger entries (measured) and emits
/// about 470 role-event bytes per member for classic accounts. The mainnet
/// limits in soroban-sdk 28's profile
/// (`soroban-sdk-28.0.0/src/testutils/cost_estimate.rs:146-151`: 200 write
/// entries, 400 ledger entries, 16,384 event bytes and 400M instructions per
/// transaction) admit 24 seats with room to spare: N = 23 writes 75 entries,
/// and the events keep more than 2,048 bytes of headroom for a classic-account
/// council (`timelock::tests::full_council_reset_resources_stay_within_mainnet_limits`).
pub const MAX_CANCELLERS: u32 = 24;
