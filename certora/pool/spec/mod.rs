mod fixture;

#[cfg(any(
    not(feature = "certora-focused"),
    feature = "certora-core-sanity-rules"
))]
pub mod core_sanity_rules;
#[cfg(any(
    not(feature = "certora-focused"),
    feature = "certora-fee-strategy-accounting-rules"
))]
pub mod fee_strategy_accounting_rules;
#[cfg(any(
    not(feature = "certora-focused"),
    feature = "certora-flash-loan-accounting-rules"
))]
pub mod flash_loan_accounting_rules;
#[cfg(any(not(feature = "certora-focused"), feature = "certora-guard-rules"))]
pub mod guard_rules;
#[cfg(any(
    not(feature = "certora-focused"),
    feature = "certora-isomorphism-rules"
))]
pub mod isomorphism_rules;
#[cfg(any(not(feature = "certora-focused"), feature = "certora-lifecycle-rules"))]
pub mod lifecycle_rules;
#[cfg(any(
    not(feature = "certora-focused"),
    feature = "certora-position-accounting-rules"
))]
pub mod position_accounting_rules;
#[cfg(any(
    not(feature = "certora-focused"),
    feature = "certora-seize-settle-accounting-rules"
))]
pub mod seize_settle_accounting_rules;
#[cfg(any(
    not(feature = "certora-focused"),
    feature = "certora-state-invariant-rules"
))]
pub mod state_invariant_rules;
