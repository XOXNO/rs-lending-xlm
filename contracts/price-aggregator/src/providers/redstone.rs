//! Adapts the RedStone price source to the aggregator's provider interface.
//! Price reads go straight to `multi_feed::read_multi_feed_source`.

use common::errors::OracleError;
use common::oracle::providers::redstone::REDSTONE_DECIMALS;
use soroban_sdk::{assert_with_error, Env};

/// Panics with `OracleError::InvalidOracleDecimals` unless `decimals`
/// equals `REDSTONE_DECIMALS`.
pub(crate) fn attest(env: &Env, decimals: u32) {
    assert_with_error!(
        env,
        decimals == REDSTONE_DECIMALS,
        OracleError::InvalidOracleDecimals
    );
}
