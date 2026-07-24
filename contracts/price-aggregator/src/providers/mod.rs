//! Provider dispatch: hard `read_required_source` (reverts) vs soft
//! `try_read_source` (`None` for per-asset read problems).

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
) -> OracleObservation {
    let observation = match source {
        OracleSourceConfig::Reflector(config) => {
            reflector::read_reflector_source(cache, config, false)
        }
        OracleSourceConfig::RedStone(config) | OracleSourceConfig::Xoxno(config) => {
            multi_feed::read_multi_feed_source(cache, config, false)
        }
    };
    observation.unwrap_or_else(|| match source {
        OracleSourceConfig::Reflector(_) => {
            panic_with_error!(cache.env(), OracleError::NoLastPrice)
        }
        OracleSourceConfig::RedStone(_) | OracleSourceConfig::Xoxno(_) => {
            panic_with_error!(cache.env(), GenericError::InvalidTicker)
        }
    })
}

/// Soft provider read for diagnostic views; `None` for any per-asset read
/// problem (missing feed, missing/short TWAP history, unresolvable quote leg).
pub(crate) fn try_read_source(
    cache: &mut ResolutionContext,
    source: &OracleSourceConfig,
) -> Option<OracleObservation> {
    match source {
        OracleSourceConfig::Reflector(config) => {
            reflector::read_reflector_source(cache, config, true)
        }
        OracleSourceConfig::RedStone(config) | OracleSourceConfig::Xoxno(config) => {
            multi_feed::read_multi_feed_source(cache, config, true)
        }
    }
}

/// Hard provider read for compose; reverts when the feed is missing or the
/// provider rejects the payload. Staleness is checked by the caller.
#[cfg(not(feature = "certora"))]
pub(crate) fn read_required_source(
    cache: &mut ResolutionContext,
    source: &OracleSourceConfig,
    _max_stale: u64,
) -> OracleObservation {
    // `max_stale` stays in the signature for certora summary parity; freshness
    // is enforced by the compose caller via `require_fresh`.
    dispatch_required_source(cache, source)
}

#[cfg(feature = "certora")]
cvlr_soroban_macros::apply_summary!(
    crate::spec::summaries::read_required_source_summary,
    pub(crate) fn read_required_source(
        cache: &mut ResolutionContext,
        source: &OracleSourceConfig,
        _max_stale: u64,
    ) -> OracleObservation {
        dispatch_required_source(cache, source)
    }
);
