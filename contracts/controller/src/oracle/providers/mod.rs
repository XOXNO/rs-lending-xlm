//! Dispatch from `OracleSourceConfig` to a provider reader.

pub(crate) mod redstone;
pub(crate) mod reflector;

use common::errors::OracleError;
use common::types::OracleSourceConfig;
use soroban_sdk::panic_with_error;

use super::observation::OracleObservation;
use crate::cache::Cache;

pub(crate) fn read_source(
    cache: &mut Cache,
    source: &OracleSourceConfig,
    max_stale: u64,
    required: bool,
) -> Option<OracleObservation> {
    let observation = match source {
        OracleSourceConfig::Reflector(config) => {
            reflector::read_reflector_source(cache, config, max_stale, required)
        }
        OracleSourceConfig::RedStone(config) => {
            redstone::read_redstone_source(cache, config, required)
        }
    };

    if required && observation.is_none() {
        panic_with_error!(cache.env(), OracleError::NoLastPrice);
    }
    observation
}
