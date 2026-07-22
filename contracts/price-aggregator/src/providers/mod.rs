//! Provider dispatch for required oracle sources.

pub(crate) mod multi_feed;
pub(crate) mod reflector;

use common::errors::{GenericError, OracleError};
use common::types::OracleSourceConfig;
use soroban_sdk::panic_with_error;

use crate::context::ResolutionContext;
use crate::observation::OracleObservation;

fn dispatch_required_source(
    cache: &mut ResolutionContext,
    source: &OracleSourceConfig,
    max_stale: u64,
) -> OracleObservation {
    try_read_source(cache, source, max_stale).unwrap_or_else(|| match source {
        OracleSourceConfig::Reflector(_) => {
            panic_with_error!(cache.env(), OracleError::NoLastPrice)
        }
        OracleSourceConfig::RedStone(_) | OracleSourceConfig::Xoxno(_) => {
            panic_with_error!(cache.env(), GenericError::InvalidTicker)
        }
    })
}

/// Soft provider read for diagnostic views; `None` when the feed is missing.
pub(crate) fn try_read_source(
    cache: &mut ResolutionContext,
    source: &OracleSourceConfig,
    max_stale: u64,
) -> Option<OracleObservation> {
    match source {
        OracleSourceConfig::Reflector(config) => {
            reflector::read_reflector_source(cache, config, max_stale)
        }
        OracleSourceConfig::RedStone(config) | OracleSourceConfig::Xoxno(config) => {
            multi_feed::read_multi_feed_source(cache, config)
        }
    }
}

#[cfg(not(feature = "certora"))]
pub(crate) fn read_required_source(
    cache: &mut ResolutionContext,
    source: &OracleSourceConfig,
    max_stale: u64,
) -> OracleObservation {
    dispatch_required_source(cache, source, max_stale)
}

#[cfg(feature = "certora")]
cvlr_soroban_macros::apply_summary!(
    crate::spec::summaries::read_required_source_summary,
    pub(crate) fn read_required_source(
        cache: &mut ResolutionContext,
        source: &OracleSourceConfig,
        max_stale: u64,
    ) -> OracleObservation {
        dispatch_required_source(cache, source, max_stale)
    }
);
