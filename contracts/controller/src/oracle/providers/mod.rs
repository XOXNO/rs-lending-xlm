//! Dispatch from `OracleSourceConfig` to a provider reader, and the single owner
//! of the required-vs-optional source decision.
//!
//! Providers (`reflector`, `redstone`) return `Some(observation)` or `None`
//! without deciding whether absence is fatal. `read_source` is the optional read;
//! `read_required_source` reverts with the source-specific error when a feed the
//! flow requires is absent.

pub(crate) mod redstone;
pub(crate) mod reflector;

use common::errors::{GenericError, OracleError};
use controller_interface::types::OracleSourceConfig;
use soroban_sdk::panic_with_error;

use super::observation::OracleObservation;
use crate::cache::Cache;

/// Reads a configured price source; `None` when the feed is absent.
pub(crate) fn read_source(
    cache: &mut Cache,
    source: &OracleSourceConfig,
    max_stale: u64,
) -> Option<OracleObservation> {
    match source {
        OracleSourceConfig::Reflector(config) => {
            reflector::read_reflector_source(cache, config, max_stale)
        }
        OracleSourceConfig::RedStone(config) => redstone::read_redstone_source(cache, config),
    }
}

/// Reads a source the flow requires, reverting with the source-specific error
/// when it is absent: a missing RedStone feed reverts `InvalidTicker`, a missing
/// Reflector feed reverts `NoLastPrice`.
pub(crate) fn read_required_source(
    cache: &mut Cache,
    source: &OracleSourceConfig,
    max_stale: u64,
) -> OracleObservation {
    read_source(cache, source, max_stale).unwrap_or_else(|| match source {
        OracleSourceConfig::RedStone(_) => {
            panic_with_error!(cache.env(), GenericError::InvalidTicker)
        }
        OracleSourceConfig::Reflector(_) => {
            panic_with_error!(cache.env(), OracleError::NoLastPrice)
        }
    })
}
