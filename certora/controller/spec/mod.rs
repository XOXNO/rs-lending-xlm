//! Controller Certora rules (`certora` feature only). One module per verification domain.

#[cfg(any(
    not(feature = "certora-focused"),
    feature = "certora-account-isolation-rules"
))]
pub mod account_isolation_rules;
#[cfg(any(not(feature = "certora-focused"), feature = "certora-boundary-rules"))]
pub mod boundary_rules;
pub mod compat;
#[cfg(any(
    not(feature = "certora-focused"),
    feature = "certora-consistency-rules"
))]
pub mod consistency_rules;
pub mod fixture;
#[cfg(any(not(feature = "certora-focused"), feature = "certora-flash-loan-rules"))]
pub mod flash_loan_rules;
pub mod health_ghost;
#[cfg(any(not(feature = "certora-focused"), feature = "certora-health-rules"))]
pub mod health_rules;
#[cfg(any(not(feature = "certora-focused"), feature = "certora-hf-lemma-rules"))]
pub mod hf_lemma_rules;
#[cfg(any(not(feature = "certora-focused"), feature = "certora-index-rules"))]
pub mod index_rules;
#[cfg(any(not(feature = "certora-focused"), feature = "certora-interest-rules"))]
pub mod interest_rules;
#[cfg(any(
    not(feature = "certora-focused"),
    feature = "certora-liquidation-rules"
))]
pub mod liquidation_rules;
#[cfg(any(
    not(feature = "certora-focused"),
    feature = "certora-market-guard-rules"
))]
pub mod market_guard_rules;
#[cfg(any(not(feature = "certora-focused"), feature = "certora-math-rules"))]
pub mod math_rules;
#[cfg(any(not(feature = "certora-focused"), feature = "certora-position-rules"))]
pub mod position_rules;
#[cfg(any(not(feature = "certora-focused"), feature = "certora-solvency-rules"))]
pub mod solvency_rules;
#[cfg(any(not(feature = "certora-focused"), feature = "certora-spoke-rules"))]
pub mod spoke_rules;
#[cfg(any(not(feature = "certora-focused"), feature = "certora-strategy-rules"))]
pub mod strategy_rules;

#[path = "../../shared/summaries/mod.rs"]
pub mod summaries;
