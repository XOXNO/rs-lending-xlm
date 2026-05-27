#![no_std]
#![allow(clippy::too_many_arguments)]

//! User-facing controller for account state, risk checks, oracle policy,
//! per-asset pool coordination, strategies, flash loans, and admin config.
//!
//! Pools custody a single asset and maintain interest indexes; the controller
//! owns account storage, collateral rules, health-factor checks, and every
//! external position mutation.
//!
//! Mutating flows construct a `ControllerCache` with an entrypoint-specific
//! `OraclePolicy`, record position and market updates during the transaction,
//! then emit batched events after storage is written.

mod access;
pub(crate) mod cache;
mod config;
pub(crate) mod cross_contract;
#[cfg(not(feature = "certora"))]
pub(crate) mod helpers;
#[cfg(feature = "certora")]
#[path = "../../../verification/certora/controller/harness/helpers.rs"]
pub(crate) mod helpers;
// Certora replaces helper math with summaries to keep position-flow proofs
// focused on controller state transitions.
pub(crate) mod emode;
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
// CVLR rules compile only under `certora`; production builds expose no spec API.

use soroban_sdk::{contract, contractmeta};

contractmeta!(key = "name", val = "Lending Controller");
contractmeta!(key = "binver", val = env!("CARGO_PKG_VERSION"));
contractmeta!(
    key = "repo",
    val = "https://github.com/xoxno/rs-lending-xlm"
);

#[contract]
pub struct Controller;
