#![no_std]

//! Governance contract. Owns the lending controller, validates every admin
//! input, and forwards accepted configuration to the controller's thin
//! owner-gated setters.

mod access;
mod deploy;
mod events;
mod forward;
mod storage;
mod validate;

#[cfg(test)]
mod tests;

use soroban_sdk::{contract, contractmeta};

contractmeta!(key = "name", val = "Lending Governance");
contractmeta!(key = "binver", val = env!("CARGO_PKG_VERSION"));
contractmeta!(
    key = "repo",
    val = "https://github.com/xoxno/rs-lending-xlm"
);

#[contract]
pub struct Governance;
