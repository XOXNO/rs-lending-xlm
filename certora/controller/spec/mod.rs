//! Controller Certora rules (`certora` feature only). One module per verification domain.

pub mod boundary_rules;
pub mod compat;
pub mod consistency_rules;
pub mod emode_rules;
pub mod flash_loan_rules;
pub mod health_ghost;
pub mod health_rules;
pub mod index_rules;
pub mod interest_rules;
pub mod liquidation_rules;
pub mod market_guard_rules;
pub mod math_rules;
pub mod oracle_compose_rules;
pub mod oracle_rules;
pub mod position_rules;
pub mod solvency_rules;
pub mod strategy_rules;
pub mod tolerance_math_rules;

#[path = "../../shared/summaries/mod.rs"]
pub mod summaries;
