//! Adapts the XOXNO price source to the aggregator's provider interface,
//! delegating price reads to the shared multi-feed reader.

use common::errors::OracleError;
use common::oracle::providers::reflector::reflector_decimals;
use common::oracle::providers::xoxno::max_submission_age;
use common::types::MultiFeedRef;
use soroban_sdk::{assert_with_error, Env};

use crate::observation::OracleObservation;
use crate::providers::multi_feed;
use crate::session::Session;

/// Validates that `feed` is configured consistently with `decimals` and
/// `max_stale`. Checks that the feed's reported decimals equal `decimals`
/// and that `max_stale` covers the feed contract's own maximum submission
/// age. Panics with `OracleError::InvalidOracleDecimals` or
/// `OracleError::InvalidStalenessConfig` if either check fails.
pub(crate) fn attest(env: &Env, feed: &MultiFeedRef, decimals: u32, max_stale: u64) {
    assert_with_error!(
        env,
        reflector_decimals(env, &feed.contract) == decimals,
        OracleError::InvalidOracleDecimals
    );
    assert_with_error!(
        env,
        max_stale >= max_submission_age(env, &feed.contract),
        OracleError::InvalidStalenessConfig
    );
}

/// Reads `feed`'s price from XOXNO via the shared multi-feed reader, scaled
/// to `decimals`. Returns `None` if the price cannot be read.
pub(crate) fn read(
    session: &mut Session,
    feed: &MultiFeedRef,
    decimals: u32,
) -> Option<OracleObservation> {
    multi_feed::read_multi_feed_source(session, feed, decimals)
}
