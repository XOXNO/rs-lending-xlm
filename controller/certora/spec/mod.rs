pub mod compat;
/// Certora Sunbeam formal verification specs for the controller.
/// Compiled only under the `certora` feature.
///
/// Modules:
///   - `health_rules`      health factor invariants
///   - `index_rules`       index monotonicity and safety
///   - `interest_rules`    rate model and compound-interest invariants
///   - `position_rules`    position add/remove consistency
///   - `isolation_rules`   isolation and e-mode constraints
///   - `flash_loan_rules`  reentrancy and repayment invariants
///   - `liquidation_rules` bonus, seizure, bad-debt invariants
///   - `oracle_rules`      staleness, tolerance bands, cache consistency
///   - `boundary_rules`    boundary conditions and overflow safety
///   - `solvency_rules`    pool solvency and scaled-amount conservation
///   - `summaries/`        function abstractions for prover feasibility
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
