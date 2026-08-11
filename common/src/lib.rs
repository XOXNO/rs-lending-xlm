//! Shared library for the lending contracts in this workspace: common types,
//! contract error codes, fixed-point math and interest-rate-curve utilities,
//! oracle validation logic, and helper functions for token transfers, TTL
//! renewal, collection deduplication, and input validation.
#![no_std]

pub mod collections;
pub mod constants;
pub mod errors;
pub mod math;
pub mod oracle;
pub mod rates;
pub mod token;
pub mod ttl;
pub mod types;
pub mod validation;

#[cfg(feature = "certora")]
#[path = "../../certora/common/spec/mod.rs"]
pub mod spec;
