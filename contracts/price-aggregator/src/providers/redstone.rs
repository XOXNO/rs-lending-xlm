use common::errors::OracleError;
use common::oracle::providers::redstone::REDSTONE_DECIMALS;
use common::types::MultiFeedRef;
use soroban_sdk::{assert_with_error, Env};

use crate::observation::OracleObservation;
use crate::providers::multi_feed;
use crate::session::Session;

pub(crate) fn attest(env: &Env, decimals: u32) {
    assert_with_error!(
        env,
        decimals == REDSTONE_DECIMALS,
        OracleError::InvalidOracleDecimals
    );
}

pub(crate) fn read(
    session: &mut Session,
    feed: &MultiFeedRef,
    decimals: u32,
) -> Option<OracleObservation> {
    multi_feed::read_multi_feed_source(session, feed, decimals)
}
