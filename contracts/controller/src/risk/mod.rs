//! Risk-engine entry point for the controller contract. Re-exports the
//! sub-modules that compute account-level collateral and debt totals
//! (`totals`), refresh per-position LTV and liquidation parameters
//! (`params`), and enforce solvency and position-limit gates
//! (`validation`).

pub(crate) mod params;
pub(crate) mod totals;
pub(crate) mod validation;

pub(crate) use params::*;
pub(crate) use totals::*;
