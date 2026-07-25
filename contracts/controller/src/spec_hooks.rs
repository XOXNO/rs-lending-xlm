//! Verification scaffolding for the `certora` build.
//!
//! Everything here is a no-op — or absent — in production builds. Production
//! logic calls a plainly named function and the `cfg` stays inside this module,
//! so a gate reads as ordinary control flow.

/// Records that the solvency gate ran its collateral-covers-debt check.
/// Read only by the health-gated Certora rules.
#[inline]
pub(crate) fn solvency_gate_checked() {
    #[cfg(feature = "certora")]
    crate::spec::health_ghost::set_checked();
}

#[cfg(feature = "certora")]
impl crate::context::Cache {
    /// Stands in for the production bulk index fetch: the harness supplies
    /// market indexes directly, so there is nothing to pull from the pool.
    pub(crate) fn fetch_market_indexes(
        &mut self,
        _hub_assets: &soroban_sdk::Vec<crate::types::HubAssetKey>,
    ) {
    }
}
