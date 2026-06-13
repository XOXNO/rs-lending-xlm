#![no_std]

//! Governance contract. Owns the lending controller, validates every admin
//! input, and forwards accepted configuration to the controller's thin
//! owner-gated setters.

mod access;
mod constants;
mod deploy;
mod events;
mod forward;
mod storage;
mod timelock;
mod validate;

#[cfg(test)]
mod tests;

use soroban_sdk::{contract, contractmeta};

pub use crate::constants::TIMELOCK_MIN_DELAY_LEDGERS;
pub use stellar_governance::timelock::OperationState;

contractmeta!(key = "name", val = "Lending Governance");
contractmeta!(key = "binver", val = env!("CARGO_PKG_VERSION"));
contractmeta!(
    key = "repo",
    val = "https://github.com/xoxno/rs-lending-xlm"
);

#[contract]
pub struct Governance;
