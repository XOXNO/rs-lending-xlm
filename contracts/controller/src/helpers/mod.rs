//! Account-math helpers, split by responsibility:
//!
//! - `math`: health-factor, LTV, and debt aggregation over position maps.
//! - `account`: min-borrow-collateral gate and in-memory account lifecycle.
//! - `utils`: payment aggregation and small shared utilities.
//!
//! Price and index reads go through `Cache`, so the active
//! `OraclePolicy` remains the caller's responsibility.

mod account;
pub(crate) mod emode_caps;
mod math;
mod risk_params;
pub(crate) mod utils;

pub(crate) use account::*;
pub(crate) use emode_caps::EModeUsageContext;
pub(crate) use math::*;
pub(crate) use risk_params::*;
