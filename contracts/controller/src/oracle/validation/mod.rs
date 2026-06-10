//! Market oracle configuration validation.

pub(crate) mod config;
pub(crate) mod oracle;

// Public entry point for the oracle validation subsystem.
pub(crate) use oracle::validate_market_oracle_sources;
