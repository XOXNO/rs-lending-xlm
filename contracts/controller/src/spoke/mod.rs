//! Per-spoke cap rules and usage accounting.
//!
//! Leaf module: depends on `storage` only. Spoke/listing config memos and the
//! `Cache`-facing gates live in `context/spoke.rs`.

pub(crate) mod caps;
pub(crate) use caps::{SpokeUsageContext, UsageSide};

#[cfg(test)]
#[path = "../../tests/spoke.rs"]
mod tests;
