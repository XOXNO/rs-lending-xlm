/// Records the completed solvency check and valued positions in Certora
/// ghost state. No-op without the `certora` feature.
#[inline]
pub(crate) fn solvency_gate_checked(_account: &common::types::Account) {
    #[cfg(feature = "certora")]
    crate::spec::health_ghost::record_gate(_account);
}

#[cfg(feature = "certora")]
impl crate::context::Context {
    /// Omits the pool's cross-contract index fetch from the Certora model.
    pub(crate) fn fetch_market_indexes(
        &mut self,
        _hub_assets: &soroban_sdk::Vec<crate::types::HubAssetKey>,
    ) {
    }
}
