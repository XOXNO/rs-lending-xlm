#![no_std]
#![allow(clippy::too_many_arguments)]

//! Lending controller. Owns accounts, risk rules, oracle policy, strategies,
//! flash loans, and admin configuration.

pub mod constants;
pub mod events;

pub use controller_interface::types;

mod cache;
mod emode;
mod external;
mod governance;
mod helpers;
mod oracle;
mod positions;
mod router;
mod storage;
mod strategies;
mod validation;
mod views;

#[cfg(feature = "certora")]
#[path = "../../../certora/controller/spec/mod.rs"]
pub mod spec;

#[cfg(feature = "testing")]
pub mod test_support {
    //! White-box hooks for the verification harness. Routes through the real
    //! storage helpers so tests drive the same flash-loan guard production
    //! uses. Compiled under `testing`; excluded from production contracts.
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
