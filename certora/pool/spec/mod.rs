//! Minimal pool-core Certora suite, compiled one accounting domain at a time.
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
#[cfg(any(
    not(feature = "certora-focused"),
    feature = "certora-position-accounting-rules"
))]
pub mod position_accounting_rules;
#[cfg(any(
    not(feature = "certora-focused"),
    feature = "certora-rate-index-accounting-rules"
))]
pub mod rate_index_accounting_rules;
#[cfg(any(
    not(feature = "certora-focused"),
    feature = "certora-seize-settle-accounting-rules"
))]
pub mod seize_settle_accounting_rules;
