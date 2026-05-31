//! Market oracle configuration validation.
//!
//! - `config`: pure, side-effect-free validation of oracle config (shape, ranges, sanity bounds); no external calls.
//! - `oracle`: live probing of Reflector/RedStone oracles (decimals, resolution, lastprice, history, base currency).
//!
//! Security boundary: anything that talks to external oracles lives in `oracle.rs`.

pub(crate) mod config;
pub(crate) mod oracle;

// Public entry point for the oracle validation subsystem.
pub(crate) use oracle::validate_market_oracle_sources;
