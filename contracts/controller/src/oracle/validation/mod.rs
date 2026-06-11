//! Market oracle configuration validation.

pub(crate) mod config;
pub(crate) mod oracle;

pub(crate) use oracle::validate_market_oracle_sources;
