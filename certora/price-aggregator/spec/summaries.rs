//! Sound over-approximation of successful external provider observations.

use cvlr::cvlr_assume;
use cvlr::nondet::nondet;

use crate::context::ResolutionContext;
use crate::observation::OracleObservation;
use common::oracle::observation::{is_stale, MAX_FUTURE_SKEW_SECONDS};
use common::types::OracleSourceConfig;

pub(crate) fn read_required_source_summary(
    cache: &mut ResolutionContext,
    _source: &OracleSourceConfig,
    max_stale: u64,
) -> OracleObservation {
    let price_wad: i128 = nondet();
    let observed_at: u64 = nondet();
    let now = cache.ledger_timestamp_secs();
    cvlr_assume!(price_wad > 0);
    // Production accepts bounded future skew from external providers.
    cvlr_assume!(observed_at <= now.saturating_add(60));
    cvlr_assume!(!is_stale(now, observed_at, max_stale));

    let published_at = if nondet::<bool>() {
        let timestamp: u64 = nondet();
        cvlr_assume!(timestamp <= now.saturating_add(MAX_FUTURE_SKEW_SECONDS));
        cvlr_assume!(!is_stale(now, timestamp, max_stale));
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
