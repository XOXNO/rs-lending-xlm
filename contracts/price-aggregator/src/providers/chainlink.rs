//! Reads scalar rounds from Chainlink's Stellar Data Feeds proxy.

use common::oracle::providers::chainlink::read_latest_round;
use common::types::ChainlinkFeedRef;

use crate::observation::OracleObservation;
use crate::session::Session;

#[cfg(feature = "certora")]
pub(crate) use certora_read::read_chainlink_source;

#[cfg(feature = "certora")]
mod certora_read {
    use super::*;
    cvlr_soroban_macros::apply_summary!(
        crate::spec::summaries::read_chainlink_source_summary,
        pub(crate) fn read_chainlink_source(
            session: &mut Session,
            feed: &ChainlinkFeedRef,
            decimals: u32,
        ) -> Option<OracleObservation> {
            super::read_chainlink_source_impl(session, feed, decimals)
        }
    );
}

#[cfg(not(feature = "certora"))]
pub(crate) use read_chainlink_source_impl as read_chainlink_source;

/// Reads and normalizes the latest round for `feed`.
pub(crate) fn read_chainlink_source_impl(
    session: &mut Session,
    feed: &ChainlinkFeedRef,
    decimals: u32,
) -> Option<OracleObservation> {
    let env = session.env();
    let round = read_latest_round(env, &feed.contract, &feed.data_id, decimals)?;
    OracleObservation::from_chainlink(session.now_secs(), &round, decimals)
}
