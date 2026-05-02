#![no_std]
#![allow(clippy::too_many_arguments)]

// Conditional summary helper: under `certora`, expands a function definition
// through `cvlr_soroban_macros::apply_summary!` (redirecting public callers
// to a nondet summary in `controller/certora/spec/summaries/`). Under any
// other build, it just re-emits the function body unchanged. Lets each
// production site declare its summary indirection once without duplicating
// the real body for each cfg arm.
#[cfg(feature = "certora")]
#[doc(hidden)]
#[macro_export]
macro_rules! summarized {
    ($summary:path, $($body:tt)*) => {
        cvlr_soroban_macros::apply_summary!($summary, $($body)*);
    };
}

#[cfg(not(feature = "certora"))]
#[doc(hidden)]
#[macro_export]
macro_rules! summarized {
    ($summary:path, $($body:tt)*) => {
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
#[path = "../certora/spec/mod.rs"]
pub mod spec;

use soroban_sdk::contract;

#[contract]
pub struct Controller;

#[cfg(test)]
mod tests;
