//! Sound over-approximation of successful external provider observations.
//!
//! Freshness is **not** owned here. `compose` derives a `stale` flag per source
//! from the timestamp this summary hands back, and the renderers act on it, so
//! a summary that forced fresh timestamps would prove the staleness rules
//! vacuously. The only assumption made about time is the bounded future skew
//! production itself accepts.
//!
//! # Where these attach
//!
//! The engine reads providers through exactly two functions —
//! `reflector::read_reflector_source` and `multi_feed::read_multi_feed_source`.
//! Summarizing those two keeps every rule on composition and rendering rather
//! than descending into wire decoding and cross-contract calls, which is where
//! the interesting logic is *not*.
//!
//! Both take a `soft` flag. It is deliberately ignored: the summary always
//! returns `Some`, modelling a provider that answers. See the soundness note on
//! [`read_source_summary`].

use cvlr::cvlr_assume;
use cvlr::nondet::nondet;

use crate::context::ResolutionContext;
use crate::observation::OracleObservation;
use common::oracle::observation::MAX_FUTURE_SKEW_SECONDS;
use common::types::{RedStoneSourceConfig, ReflectorSourceConfig};

/// An arbitrary successful observation: any positive price, at any time not
/// implausibly far in the future.
///
/// # Soundness
///
/// Always `Some`, which is sound for rules over `price` / `prices`: there an
/// unreadable source only reverts, and a reverting path cannot violate a rule.
/// It is **not** sound for a rule over `price_status` / `prices_status`, which
/// answer `PriceStatus::unusable` without reverting — modelling every source as
/// readable would prove the unusable branch unreachable. Such a rule needs an
/// unsummarized read or its own conf.
fn read_source_summary(cache: &mut ResolutionContext) -> Option<OracleObservation> {
    let price_wad: i128 = nondet();
    let observed_at: u64 = nondet();
    let now = cache.ledger_timestamp_secs();
    cvlr_assume!(price_wad > 0);
    // Production accepts bounded future skew from external providers.
    cvlr_assume!(observed_at <= now.saturating_add(60));

    let published_at = if nondet::<bool>() {
        let timestamp: u64 = nondet();
        cvlr_assume!(timestamp <= now.saturating_add(MAX_FUTURE_SKEW_SECONDS));
        Some(timestamp)
    } else {
        None
    };

    Some(OracleObservation {
        price_wad,
        observed_at,
        published_at,
    })
}

pub(crate) fn read_reflector_source_summary(
    cache: &mut ResolutionContext,
    _config: &ReflectorSourceConfig,
    _soft: bool,
) -> Option<OracleObservation> {
    read_source_summary(cache)
}

pub(crate) fn read_multi_feed_source_summary(
    cache: &mut ResolutionContext,
    _config: &RedStoneSourceConfig,
    _soft: bool,
) -> Option<OracleObservation> {
    read_source_summary(cache)
}
