//! Common-layer Certora rules: fixed-point math and rate model.
pub mod harness;
#[cfg(any(not(feature = "certora-focused"), feature = "certora-math-rules"))]
pub mod math_rules;
#[cfg(any(
    not(feature = "certora-focused"),
    feature = "certora-rate-index-accounting-rules"
))]
pub mod rate_index_accounting_rules;
#[cfg(any(not(feature = "certora-focused"), feature = "certora-rates-rules"))]
pub mod rates_rules;
pub mod summaries;
