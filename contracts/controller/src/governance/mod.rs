//! Owner- and role-gated administration, split by responsibility:
//!
//! - `access`: ownership, roles (KEEPER / REVENUE / ORACLE), pause, upgrade.
//! - `config`: market, oracle, e-mode, cap, and protocol configuration.

pub(crate) mod access;
pub(crate) mod config;
