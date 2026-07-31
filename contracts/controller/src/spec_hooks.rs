#[inline]
pub(crate) fn solvency_gate_checked() {
    #[cfg(feature = "certora")]
    crate::spec::health_ghost::set_checked();
}

#[cfg(feature = "certora")]
impl crate::context::Cache {
    pub(crate) fn fetch_market_indexes(
        &mut self,
        _hub_assets: &soroban_sdk::Vec<crate::types::HubAssetKey>,
    ) {
    }
}
