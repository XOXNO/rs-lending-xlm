#![no_std]
#![allow(clippy::too_many_arguments)]

//! Lending controller. Owns accounts, risk rules, oracle policy, strategies,
//! flash loans, and admin configuration.

pub mod constants;
pub mod events;

pub use common::types;

mod account;
mod config;
mod context;
mod external;
mod governance;
mod oracle;
mod payments;
mod pool_ops;
mod positions;
mod risk;
mod setup;
mod spoke;
mod storage;
mod strategies;
mod views;

#[cfg(feature = "certora")]
#[path = "../../../certora/controller/spec/mod.rs"]
pub mod spec;

#[cfg(feature = "testing")]
pub mod test_support {
    //! White-box hooks for the verification harness.
    //! Routes through real storage helpers so tests exercise production guards.
    use crate::storage;
    use soroban_sdk::Env;

    pub fn set_flash_loan_ongoing(env: &Env, ongoing: bool) {
        storage::set_flash_loan_ongoing(env, ongoing);
    }

    #[must_use]
    pub fn is_flash_loan_ongoing(env: &Env) -> bool {
        storage::is_flash_loan_ongoing(env)
    }
}

use soroban_sdk::{contract, contractmeta};

contractmeta!(key = "name", val = "Lending Controller");
contractmeta!(key = "binver", val = env!("CARGO_PKG_VERSION"));
contractmeta!(
    key = "repo",
    val = "https://github.com/xoxno/rs-lending-xlm"
);

#[contract]
pub struct Controller;
