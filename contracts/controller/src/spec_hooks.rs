//! Hooks used only when building for Certora formal verification (`certora`
//! feature). Marks a checkpoint in the solvency-gate code path for the formal spec,
//! and, under `certora`, replaces the pool-backed market-index fetch with a no-op
//! stub so the harness does not perform live pool calls.

/// Marks the solvency gate as checked. A no-op unless the `certora` feature is
/// enabled, in which case it records the checkpoint via
/// `crate::spec::health_ghost::set_checked` (the `spec` module only exists
/// under the `certora` feature).
#[inline]
pub(crate) fn solvency_gate_checked() {
    #[cfg(feature = "certora")]
    crate::spec::health_ghost::set_checked();
}

#[cfg(feature = "certora")]
impl crate::context::Cache {
    /// Certora-only stub for [`crate::context::Cache::fetch_market_indexes`]. Does
    /// nothing; replaces the pool-backed implementation compiled in normal builds.
    pub(crate) fn fetch_market_indexes(
        &mut self,
        _hub_assets: &soroban_sdk::Vec<crate::types::HubAssetKey>,
    ) {
    }
}
