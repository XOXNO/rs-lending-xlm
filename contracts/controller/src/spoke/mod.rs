//! Per-spoke risk resolution from `SpokeAsset(spoke_id, hub_asset)`.

pub(crate) mod caps;
pub(crate) use caps::SpokeUsageContext;

use common::errors::SpokeError;
use common::types::{AssetConfig, HubAssetKey};
use soroban_sdk::{panic_with_error, Env};

use crate::context::Cache;

/// Canonical risk-entry gate: the spoke must be active (`SpokeDeprecated`) and
/// list the asset (`AssetNotInSpoke`); returns the listed risk config.
pub(crate) fn require_listed_active_config(
    env: &Env,
    cache: &mut Cache,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
) -> AssetConfig {
    cache.active_spoke(spoke_id);
    let config = cache
        .cached_spoke_asset(spoke_id, hub_asset)
        .unwrap_or_else(|| panic_with_error!(env, SpokeError::AssetNotInSpoke));
    (&config).into()
}

#[cfg(test)]
#[path = "../../tests/spoke.rs"]
mod tests;
