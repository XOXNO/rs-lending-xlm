#![no_std]

//! Governance contract for the lending protocol.
//!
//! Purpose: Owns the controller address, owns the timelock schedule, and
//! exposes owner + role gated entrypoints that validate then forward
//! configuration changes (assets, oracles, spokes, strategies) to the
//! controller.
//!
//! Structure:
//! - `storage.rs`: controller pointer and role-revocation guard keys.
//! - `access.rs`: owner/role application helpers + Governance impl for ctor/accept/has_role.
//! - `timelock.rs`: schedule/execute/cancel plus immediate guardian actions.
//! - `deploy.rs`: controller deployment helper (emits event).
//! - `op.rs`: operation applicator + resolver for controller calls.
//! - `validate/`: input validation submodules for assets, spokes, oracles, tolerances.
//! - `events.rs`: governance-specific events (deploy).
//! - `constants.rs`: timelock delay bounds.
//!
//! High-level ops are timelocked; guardian role bypasses for incidents.
//! Uses stellar-access for ownable/access-control and stellar-governance for
//! the underlying timelock.

mod access;
mod constants;
mod deploy;
mod events;
pub mod op;
mod storage;
mod timelock;
mod validate;

#[cfg(test)]
#[path = "../tests/flows.rs"]
mod tests;

#[cfg(test)]
#[path = "../tests/support.rs"]
mod test_support;

use soroban_sdk::{contract, contractmeta};

pub use crate::constants::{
    TIMELOCK_MAX_DELAY_LEDGERS, TIMELOCK_MIN_DELAY_LEDGERS, TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS,
};

pub use stellar_governance::timelock::OperationState;

contractmeta!(key = "name", val = "Lending Governance");
contractmeta!(key = "binver", val = env!("CARGO_PKG_VERSION"));
contractmeta!(
    key = "repo",
    val = "https://github.com/xoxno/rs-lending-xlm"
);

#[contract]
pub struct Governance;
