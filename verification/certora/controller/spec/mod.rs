//! Certora Sunbeam formal verification specs for the controller.
//! Compiled only under the `certora` feature.
//!
//! The module boundary mirrors the prover configuration boundary: each rule
//! module owns one verification domain, while the shared `summaries` module
//! owns external-call abstractions. Keep new rules in the smallest domain that
//! explains the safety property being proved.
//!
//! Modules:
//!   - `account_isolation_rules` cross-account non-interference
//!   - `boundary_rules`          boundary conditions and overflow safety
//!   - `compat`                  single-asset wrappers over controller entry points
//!   - `consistency_rules`       controller persists pool-returned positions
//!   - `emode_rules`             e-mode category constraints
//!   - `flash_loan_rules`        reentrancy and repayment invariants
//!   - `health_rules`            health factor invariants
//!   - `index_rules`             index monotonicity and safety
//!   - `interest_rules`          rate model and compound-interest invariants
//!   - `isolation_rules`         isolation-mode debt-ceiling constraints
//!   - `liquidation_rules`       bonus, seizure, bad-debt invariants
//!   - `market_guard_rules`      preconditions that block new supply/borrow
//!   - `math_rules`              controller-level fixed-point math
//!   - `oracle_rules`            staleness, tolerance bands, cache consistency
//!   - `position_rules`          position add/remove consistency
//!   - `solvency_rules`          pool solvency and scaled-amount conservation
//!   - `strategy_rules`          leverage/strategy invariants
//!   - shared `summaries`        function abstractions for prover feasibility

pub mod account_isolation_rules;
pub mod boundary_rules;
pub mod compat;
pub mod consistency_rules;
pub mod emode_rules;
pub mod flash_loan_rules;
pub mod health_rules;
pub mod index_rules;
pub mod interest_rules;
pub mod isolation_rules;
pub mod liquidation_rules;
pub mod market_guard_rules;
pub mod math_rules;
pub mod oracle_rules;
pub mod position_rules;
pub mod solvency_rules;
pub mod strategy_rules;

#[path = "../../shared/summaries/mod.rs"]
pub mod summaries;
