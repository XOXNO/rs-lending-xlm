#![no_std]
#![allow(clippy::too_many_arguments)]

#[cfg(not(feature = "certora"))]
mod abi;
mod access;
pub(crate) mod cache;
mod config;
pub(crate) mod cross_contract;
#[cfg(not(feature = "certora"))]
pub(crate) mod helpers;
#[cfg(feature = "certora")]
#[path = "../../../verification/certora/controller/harness/helpers.rs"]
pub(crate) mod helpers;
pub(crate) mod oracle;
pub(crate) mod positions;
mod router;
mod storage;
mod strategies;
mod utils;
mod validation;
mod views;

#[cfg(feature = "certora")]
#[path = "../../../verification/certora/controller/spec/mod.rs"]
pub mod spec;

use soroban_sdk::{contract, contractmeta};

contractmeta!(key = "name", val = "Lending Controller");
contractmeta!(key = "binver", val = env!("CARGO_PKG_VERSION"));
contractmeta!(key = "repo", val = "https://github.com/xoxno/rs-lending-xlm");

#[contract]
pub struct Controller;
