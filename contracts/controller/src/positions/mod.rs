//! Core position lifecycle operations.
//!
//! Each submodule implements one public entrypoint plus its internal
//! `process_*` pipeline. The split exists so that:
//!
//! - Supply and repay (credit-side or debt-reduction) can be optimized to
//!   touch only one side of the account storage (ADR 0002).
//! - Every flow has a single, reviewable place that declares its
//!   `OraclePolicy`, runs the exact pre- and post-flight risk gates, and
//!   records the precise event deltas.
//! - Liquidation math is isolated in its own file because it is the most
//!   subtle numeric component and is heavily exercised by both the Rust
//!   tests and the certora rules.
//!
//! All public functions are re-exported via `#[contractimpl]` blocks in the
//! individual files (the `Controller` impl is distributed across modules).
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
