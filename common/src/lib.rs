//! Shared lending primitives: math, constants, rates, oracle helpers, types,
//! errors, validation. No contract storage — consumers own TTL and persistence.
#![no_std]

pub mod constants;
pub mod errors;
pub mod math;
pub mod oracle;
pub mod rates;
pub mod types;
pub mod validation;

#[cfg(feature = "certora")]
#[path = "../../certora/common/spec/mod.rs"]
pub mod spec;
