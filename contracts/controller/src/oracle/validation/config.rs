//! Pure configuration and shape validation for oracle sources.
//!
//! Checks here never make live calls to external oracle contracts
//! (Reflector or RedStone), so they are safe to run in any context.

use common::errors::{GenericError, OracleError};
use common::types::{MarketOracleConfigInput, OracleStrategy};
#[cfg(not(feature = "testing"))]
use common::types::{OracleReadMode, OracleSourceConfigInput};
use soroban_sdk::{assert_with_error, panic_with_error, Env};

use super::super::observation::{
    MAX_ORACLE_DECIMALS, MAX_PRICE_STALE_SECONDS, MAX_TWAP_RECORDS, MIN_ORACLE_DECIMALS,
    MIN_PRICE_STALE_SECONDS,
};

pub(crate) fn validate_oracle_config_shape(env: &Env, config: &MarketOracleConfigInput) {
    let needs_anchor = config.strategy == OracleStrategy::PrimaryWithAnchor;
    let has_anchor = !config.anchor.is_none();
    assert_with_error!(
        env,
        needs_anchor == has_anchor,
        GenericError::InvalidExchangeSrc
    );

    // Primary and anchor must differ: identical sources collapse the dual feed
    // into one, so `is_within_anchor` compares a price against itself (~1.0),
    // always passes the tolerance band, and voids the diversity guarantee.
    if let Some(anchor) = config.anchor.as_ref() {
        assert_with_error!(
            env,
            config.primary != *anchor,
            GenericError::InvalidExchangeSrc
        );
    }

    // Production rejects Single + Spot (INVARIANTS §4.3, ADR-0003); a TWAP
    // or anchor cross-check is required.
    #[cfg(not(feature = "testing"))]
    {
        if matches!(config.strategy, OracleStrategy::Single)
            && matches!(
                config.primary,
                OracleSourceConfigInput::Reflector(ref r) if matches!(r.read_mode, OracleReadMode::Spot)
            )
        {
            panic_with_error!(env, GenericError::SpotOnlyNotProductionSafe);
        }
    }
}

pub(crate) fn validate_max_stale(env: &Env, max_stale: u64) {
    assert_with_error!(
        env,
        (MIN_PRICE_STALE_SECONDS..=MAX_PRICE_STALE_SECONDS).contains(&max_stale),
        OracleError::InvalidStalenessConfig
    );
}

/// Validate sanity bounds.
pub(crate) fn validate_sanity_bounds(env: &Env, min_wad: i128, max_wad: i128) {
    if min_wad <= 0 || max_wad <= 0 || min_wad >= max_wad {
        panic_with_error!(env, OracleError::InvalidSanityBounds);
    }
    assert_with_error!(
        env,
        max_wad <= common::constants::MAX_REASONABLE_PRICE_WAD,
        OracleError::InvalidSanityBounds
    );
}

pub(crate) fn validate_decimals(env: &Env, decimals: u32) {
    assert_with_error!(
        env,
        (MIN_ORACLE_DECIMALS..=MAX_ORACLE_DECIMALS).contains(&decimals),
        OracleError::InvalidOracleDecimals
    );
}

pub(crate) fn validate_twap_records(env: &Env, records: u32) {
    assert_with_error!(env, records != 0, OracleError::TwapInsufficientObservations);
    assert_with_error!(
        env,
        records <= MAX_TWAP_RECORDS,
        OracleError::InvalidOracleTokenType
    );
}
