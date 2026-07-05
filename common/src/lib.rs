//! Shared primitives for the lending pool and controller contracts.
//!
//! Houses the protocol's fixed-point math ([`math`]), numeric constants
//! ([`constants`]), interest-rate and index accrual ([`rates`]), oracle
//! plumbing ([`oracle`]), ABI-raw and typed domain types ([`types`]), stable
//! error codes ([`errors`]), and cross-contract guard checks ([`validation`]).
//! It owns no contract storage; TTL and persistence are the consumers' job.
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
