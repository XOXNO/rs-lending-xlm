//! Market oracle configuration validation.
//!
//! This module is deliberately split for clarity and maintainability:
//!
//! - `config`: Pure, side-effect-free validation of oracle configuration
//   shape, ranges, sanity bounds, etc. No external contract calls.
//! - `oracle`: All logic that performs live probing of Reflector and
//   RedStone oracles (decimals, resolution, lastprice, history checks, base
//   currency validation, etc.).
//!
//! The split makes the security boundary explicit: anything that talks to
// external oracles lives in `oracle.rs`.

pub(crate) mod config;
pub(crate) mod oracle;

// Public entry point for the oracle validation subsystem.
pub(crate) use oracle::validate_market_oracle_sources;
