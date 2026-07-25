//! Sound over-approximation of successful external provider observations.
//! Freshness is owned by the renderers — `price::require_leg` reads the `stale`
//! flag `compose` derives — not by this summary.

use cvlr::cvlr_assume;
use cvlr::nondet::nondet;

use crate::context::ResolutionContext;
use crate::observation::OracleObservation;
use common::oracle::observation::MAX_FUTURE_SKEW_SECONDS;
use common::types::OracleSourceConfig;

pub(crate) fn read_required_source_summary(
    cache: &mut ResolutionContext,
    _source: &OracleSourceConfig,
) -> OracleObservation {
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

    OracleObservation {
        price_wad,
        observed_at,
        published_at,
    }
}

/// Soft read of the same modeled observation. Every leg a successful `price`
/// call resolves comes through `providers::try_read_source`, so summarizing it
/// keeps the rules on the composition and rendering logic instead of descending
/// into provider wire decoding and cross-contract calls.
///
/// Always `Some`, which is sound for the `price` / `prices` rules: there an
/// unreadable leg only reverts, and a reverting path cannot violate a rule. It
/// is *not* sound for a rule over `price_status` / `prices_status`, which
/// answer `PriceStatus::unusable` without reverting — such a rule needs an
/// unsummarized read or its own conf.
pub(crate) fn try_read_source_summary(
    cache: &mut ResolutionContext,
    source: &OracleSourceConfig,
) -> Option<OracleObservation> {
    Some(read_required_source_summary(cache, source))
}
