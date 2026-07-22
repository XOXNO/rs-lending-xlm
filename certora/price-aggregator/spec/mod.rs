//! Price-aggregator Certora rules (`certora` feature only). Oracle soundness:
//! tolerance-band blend, staleness/sanity, and price-cache consistency now live
//! with the oracle engine that owns them.

#[cfg(any(not(feature = "certora-focused"), feature = "certora-freshness-rules"))]
pub mod freshness_rules;
#[cfg(any(not(feature = "certora-focused"), feature = "certora-oracle-rules"))]
pub mod oracle_rules;
pub mod summaries;
#[cfg(any(
    not(feature = "certora-focused"),
    feature = "certora-tolerance-math-rules"
))]
pub mod tolerance_math_rules;
