pub mod compat;
/// Certora Sunbeam formal verification specs for the rs-lending controller.
///
/// These modules contain `#[rule]` functions verified by the Certora Prover.
/// They are only compiled when the `certora` feature is enabled.
///
/// Structure mirrors Blend v2's Certora setup:
///   - `model`          — ghost state, skolem variables, helpers
///   - `health_rules`   — health factor invariants
///   - `index_rules`    — index monotonicity & safety
///   - `interest_rules` — interest rate model & compound interest invariants
///   - `position_rules` — position integrity (add/remove consistency)
///   - `isolation_rules` — isolation mode & e-mode constraints
///   - `flash_loan_rules` — reentrancy & repayment invariants
///   - `liquidation_rules` — liquidation bonus, seizure, bad debt invariants
///   - `oracle_rules`   — price staleness, tolerance bands, cache consistency
///   - `boundary_rules` — exact boundary conditions, off-by-one, overflow safety
///   - `solvency_rules` — pool solvency, zero-amount reverts, scaled conservation
///   - `summaries/`     — function abstractions for prover feasibility
pub mod model;

pub mod boundary_rules;
pub mod emode_rules;
pub mod flash_loan_rules;
pub mod health_rules;
pub mod index_rules;
pub mod interest_rules;
pub mod isolation_rules;
pub mod liquidation_rules;
pub mod math_rules;
pub mod oracle_rules;
pub mod position_rules;
pub mod solvency_rules;
pub mod strategy_rules;

pub mod summaries;
