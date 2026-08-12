//! Spoke usage-cap module. Declares the `caps` submodule and re-exports
//! `SpokeUsageContext` and `UsageSide` for tracking scaled spoke usage and enforcing
//! per-spoke supply/borrow caps.

pub(crate) mod caps;
pub(crate) use caps::{SpokeUsageContext, UsageSide};

#[cfg(test)]
#[path = "../../tests/spoke.rs"]
mod tests;
