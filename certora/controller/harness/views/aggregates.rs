//! Certora harness substitute for `controller::views_aggregates`.
//!
//! Re-exports the existing USD-aggregate summaries from
//! `certora/shared/summaries/mod.rs` under the production
//! names. Every aggregate is replaced by a bounded nondet i128 so the
//! prover doesn't have to traverse per-asset iteration during view
//! verification.

pub use crate::spec::summaries::{
    ltv_collateral_in_usd_summary as ltv_collateral_in_usd,
    total_borrow_in_usd_summary as total_borrow_in_usd,
    total_collateral_in_usd_summary as total_collateral_in_usd,
};
