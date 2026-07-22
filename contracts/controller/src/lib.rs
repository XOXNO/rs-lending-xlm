#![no_std]
#![allow(clippy::too_many_arguments)]

//! Lending controller. Owns accounts, risk rules, price-aggregator pricing,
//! strategies, flash loans, and admin configuration.
//!
//! Top level only declares modules; business logic lives in the submodules
//! following mod.rs + storage.rs (where state owned) layout.

pub mod constants;
pub mod events;

pub use common::types;

mod account;
mod config;
mod context;
mod external;
mod governance;
mod payments;
mod pool_ops;
mod positions;
mod risk;
mod spoke;
mod storage;
mod strategies;
mod views;

#[cfg(feature = "certora")]
#[path = "../../../certora/controller/spec/mod.rs"]
pub mod spec;

#[cfg(feature = "testing")]
#[path = "../tests/test_support.rs"]
pub mod test_support;

use soroban_sdk::{contract, contractmeta};

contractmeta!(key = "name", val = "Lending Controller");
contractmeta!(key = "binver", val = env!("CARGO_PKG_VERSION"));
contractmeta!(
    key = "repo",
    val = "https://github.com/xoxno/rs-lending-xlm"
);

#[contract]
pub struct Controller;
