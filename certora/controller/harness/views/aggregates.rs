//! Certora harness for `controller::views_aggregates`.
//! Bounded nondet USD aggregates (no per-asset iteration).

pub use crate::spec::summaries::{
    ltv_collateral_in_usd_summary as ltv_collateral_in_usd,
    total_borrow_in_usd_summary as total_borrow_in_usd,
    total_collateral_in_usd_summary as total_collateral_in_usd,
};
