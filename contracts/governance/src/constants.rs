pub const TIMELOCK_MIN_DELAY_LEDGERS: u32 = 34_560;

pub const TIMELOCK_MAX_DELAY_LEDGERS: u32 = 241_920;

// TEMPORARY until audits conclude: production value is 120_960 (~7 days).
// Dropped to 12 pre-audit so wasm upgrades, ownership transfers, aggregator
// re-point, role grants, and force-socialization ship without the week wait.
// Restore 120_960 via a governance-executed `UpgradeGov` once audits close.
pub const TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS: u32 = 12;

pub const TIMELOCK_OPERATION_GRACE_LEDGERS: u32 = 120_960;

pub const TIMELOCK_RECOVERY_MIN_DELAY_LEDGERS: u32 = 518_400;
