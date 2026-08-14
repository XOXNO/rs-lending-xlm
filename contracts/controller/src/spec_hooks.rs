/// Marks the solvency check as having run and snapshots the position book it
/// valued, for the Certora harness's ghost state; a no-op unless built with
/// the `certora` feature.
#[inline]
pub(crate) fn solvency_gate_checked(_account: &common::types::Account) {
    #[cfg(feature = "certora")]
    crate::spec::health_ghost::record_gate(_account);
}

#[cfg(feature = "certora")]
impl crate::context::Cache {
    /// No-op stand-in for the normal market-index fetch, compiled in under
    /// the `certora` feature so the formal-verification harness does not
    /// model the pool's cross-contract index call.
    pub(crate) fn fetch_market_indexes(
        &mut self,
        _hub_assets: &soroban_sdk::Vec<crate::types::HubAssetKey>,
    ) {
    }
}
