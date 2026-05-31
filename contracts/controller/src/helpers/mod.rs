//! Account-math helpers, split by responsibility:
//!
//! - `math`: health-factor, LTV, and debt aggregation over position maps.
//! - `account`: per-asset dust gates and in-memory account lifecycle.
//!
//! Price and index reads go through `Cache`, so the active
//! `OraclePolicy` remains the caller's responsibility.

mod account;
mod math;

pub(crate) use account::*;
pub(crate) use math::*;
