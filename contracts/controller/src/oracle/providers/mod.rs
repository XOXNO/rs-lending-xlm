//! Dispatch from `OracleSourceConfig` to a provider reader, and the single owner
//! of the source-required revert.
//!
//! Providers (`reflector`, `redstone`) return `Some(observation)` or `None`
//! without deciding whether absence is fatal. `read_required_source` reads the
//! configured provider and reverts with that source's error when the feed is
//! absent: a missing Reflector feed reverts `NoLastPrice`, a missing RedStone
//! feed reverts `InvalidTicker`.

pub(crate) mod redstone;
pub(crate) mod reflector;

use common::errors::{GenericError, OracleError};
use common::types::OracleSourceConfig;
use soroban_sdk::panic_with_error;

use super::observation::OracleObservation;
use crate::context::Cache;

/// Reads a source the flow requires, reverting with the source-specific error
/// when it is absent: a missing RedStone feed reverts `InvalidTicker`, a missing
/// Reflector feed reverts `NoLastPrice`.
pub(crate) fn read_required_source(
    cache: &mut Cache,
    source: &OracleSourceConfig,
    max_stale: u64,
) -> OracleObservation {
    match source {
        OracleSourceConfig::Reflector(config) => {
            reflector::read_reflector_source(cache, config, max_stale)
                .unwrap_or_else(|| panic_with_error!(cache.env(), OracleError::NoLastPrice))
        }
        OracleSourceConfig::RedStone(config) => redstone::read_redstone_source(cache, config)
            .unwrap_or_else(|| panic_with_error!(cache.env(), GenericError::InvalidTicker)),
    }
}
