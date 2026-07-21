//! Provider dispatch for required oracle sources.

pub(crate) mod redstone;
pub(crate) mod reflector;

use common::errors::{GenericError, OracleError};
use common::types::OracleSourceConfig;
use soroban_sdk::panic_with_error;

use crate::context::PriceCache as Cache;
use crate::observation::OracleObservation;

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
        OracleSourceConfig::RedStone(config) | OracleSourceConfig::Xoxno(config) => {
            redstone::read_redstone_source(cache, config)
                .unwrap_or_else(|| panic_with_error!(cache.env(), GenericError::InvalidTicker))
        }
    }
}
