use cvlr::cvlr_assume;
use cvlr::nondet::{nondet, nondet_option};

use crate::observation::OracleObservation;
use crate::session::Session;
use common::oracle::observation::MAX_FUTURE_SKEW_SECONDS;
use common::types::{MultiFeedRef, ReflectorFeedRef};

fn read_source_summary(session: &mut Session) -> Option<OracleObservation> {
    nondet_option(|| {
        let price_wad: i128 = nondet();
        let timestamp: u64 = nondet();
        let now = session.now_secs();
        cvlr_assume!(price_wad > 0);
        cvlr_assume!(timestamp <= now.saturating_add(MAX_FUTURE_SKEW_SECONDS));

        OracleObservation {
            price_wad,
            timestamp,
        }
    })
}

pub(crate) fn read_reflector_source_summary(
    session: &mut Session,
    _feed: &ReflectorFeedRef,
    _decimals: u32,
) -> Option<OracleObservation> {
    read_source_summary(session)
}

pub(crate) fn read_multi_feed_source_summary(
    session: &mut Session,
    _feed: &MultiFeedRef,
    _decimals: u32,
) -> Option<OracleObservation> {
    read_source_summary(session)
}
