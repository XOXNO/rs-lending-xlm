//! Core position lifecycle operations.
//!
//! Each submodule implements one public entrypoint plus its internal `process_*`
//! pipeline. The split lets supply and repay touch only one side of account
//! storage (ADR 0002), gives every flow a single reviewable place for its
//! `OraclePolicy` + risk gates + event deltas, and isolates the subtle, heavily
//! tested liquidation math in its own file. Public functions are re-exported via
//! per-file `#[contractimpl]` blocks (the `Controller` impl is distributed).
//!
//! Common stages in every flow:
//! 1. Auth + flash-loan guard
//! 2. Cache construction with the flow-appropriate `OraclePolicy`
//! 3. Account resolution / creation
//! 4. Validation (limits, health, LTV, caps, dust, … via `validation`)
//! 5. Pool calls via `cross_contract::pool`
//! 6. Post-mutation health / LTV re-check where risk increased
//! 7. Storage write + batch event recording on the cache
//!
//! See the top-level crate docs and the individual files for the exact
//! policy and checks used by each operation.

pub mod borrow;
pub mod isolated_debt;
pub mod liquidation;
pub mod liquidation_math;
pub mod repay;
pub mod supply;
pub mod withdraw;
