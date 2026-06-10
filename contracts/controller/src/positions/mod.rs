//! Core position lifecycle operations.
//!
//! Each submodule owns one public position flow and its `process_*` pipeline.
//! Shared stages are auth, cache setup, account resolution, validation, pool
//! calls, post-checks, storage writes, and event recording.

pub mod borrow;
pub mod isolated_debt;
pub mod liquidation;
pub mod liquidation_math;
pub mod repay;
pub mod supply;
pub mod withdraw;
