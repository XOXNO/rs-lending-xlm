#![no_std]
#![allow(clippy::too_many_arguments)]

#[cfg(feature = "certora")]
#[path = "../../verification/certora/controller/harness/summarized.rs"]
mod summarized;

#[cfg(not(feature = "certora"))]
#[doc(hidden)]
#[macro_export]
macro_rules! summarized {
    ($($summary:ident)::+, $($body:tt)*) => {
        $($body)*
    };
}

mod access;
pub(crate) mod cache;
mod config;
mod flash_loan;
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
#[path = "../../verification/certora/controller/spec/mod.rs"]
pub mod spec;

use soroban_sdk::contract;

#[contract]
pub struct Controller;
