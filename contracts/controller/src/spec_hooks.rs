use common::types::Account;
#[cfg(feature = "certora")]
use common::types::HubAssetKey;
#[cfg(feature = "certora")]
use soroban_sdk::Vec;

#[cfg(feature = "certora")]
use crate::context::Context;
#[cfg(feature = "certora")]
use crate::spec::health_ghost;

/// Records the completed solvency check and valued positions in Certora
/// ghost state. No-op without the `certora` feature.
#[inline]
pub(crate) fn solvency_gate_checked(_account: &Account) {
    #[cfg(feature = "certora")]
    health_ghost::record_gate(_account);
}

#[cfg(feature = "certora")]
impl Context {
    /// Omits the pool's cross-contract index fetch from the Certora model.
    pub(crate) fn fetch_market_indexes(&mut self, _hub_assets: &Vec<HubAssetKey>) {}
}
