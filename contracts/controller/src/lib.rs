#![no_std]
#![allow(clippy::too_many_arguments)]

mod access;
pub(crate) mod cache;
mod config;
pub(crate) mod cross_contract;
mod flash_loan;
#[cfg(not(feature = "certora"))]
pub(crate) mod helpers;
#[cfg(feature = "certora")]
#[path = "../../../verification/certora/controller/harness/helpers.rs"]
pub(crate) mod helpers;
pub(crate) mod oracle;
pub(crate) mod positions;
mod router;
mod storage;
mod strategy;
mod utils;
mod validation;
mod views;

#[cfg(feature = "certora")]
#[path = "../../../verification/certora/controller/spec/mod.rs"]
pub mod spec;

use soroban_sdk::contract;

#[contract]
pub struct Controller;
