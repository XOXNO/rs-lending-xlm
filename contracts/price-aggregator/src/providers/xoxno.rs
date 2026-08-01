use common::errors::OracleError;
use common::oracle::providers::reflector::reflector_decimals;
use common::oracle::providers::xoxno::max_submission_age;
use common::types::MultiFeedRef;
use soroban_sdk::{assert_with_error, Env};

use crate::observation::OracleObservation;
use crate::providers::multi_feed;
use crate::session::Session;

pub(crate) fn attest(env: &Env, feed: &MultiFeedRef, decimals: u32, max_stale: u64) {
    // The Xoxno adapter implements the SEP-40 `decimals()` getter, so the shared
    // reflector reader attests it here.
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

pub(crate) fn read(
    session: &mut Session,
    feed: &MultiFeedRef,
    decimals: u32,
) -> Option<OracleObservation> {
    multi_feed::read_multi_feed_source(session, feed, decimals)
}
