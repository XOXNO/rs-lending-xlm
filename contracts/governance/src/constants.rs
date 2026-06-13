//! Governance constants.

/// Minimum timelock delay in LEDGERS. 48h at the Stellar ~5s/ledger close
/// time (= 2 x OZ DAY_IN_LEDGERS of 17280). The deploy constructor takes the
/// delay as a parameter so non-mainnet networks can arm a shorter value for
/// live end-to-end exercise; this is the mainnet invariant we commit to.
pub const TIMELOCK_MIN_DELAY_LEDGERS: u32 = 34_560;
