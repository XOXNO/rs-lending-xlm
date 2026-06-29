//! Owner- and role-gated administration, split by responsibility:
//!
//! - `access`: ownership, pause, upgrade.
//! - `config`: market, oracle, spoke, cap, and protocol configuration.

pub(crate) mod access;
pub(crate) mod config;
