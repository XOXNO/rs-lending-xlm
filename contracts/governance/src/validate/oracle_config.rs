//! Pure oracle config validation. No external oracle calls.

use common::errors::{GenericError, OracleError};
use common::oracle::observation::{
    MAX_ORACLE_DECIMALS, MAX_PRICE_STALE_SECONDS, MAX_TWAP_RECORDS, MIN_ORACLE_DECIMALS,
    MIN_PRICE_STALE_SECONDS,
};
use common::types::{MarketOracleConfigInput, OracleStrategy};
use common::validation::validate_sanity_bounds as common_validate_sanity_bounds;

#[cfg(not(feature = "testing"))]
use common::types::{OracleReadMode, OracleSourceConfigInput};
#[cfg(not(feature = "testing"))]
use soroban_sdk::panic_with_error;

use soroban_sdk::{assert_with_error, Env};

// ################## LOW-LEVEL HELPERS ##################

/// Validates oracle shape without live calls.
pub(crate) fn validate_oracle_config_shape(env: &Env, config: &MarketOracleConfigInput) {
    let needs_anchor = config.strategy == OracleStrategy::PrimaryWithAnchor;
    let has_anchor = !config.anchor.is_none();
    assert_with_error!(
        env,
        needs_anchor == has_anchor,
        GenericError::InvalidExchangeSrc
    );

    // Primary and anchor must read different underlying feeds.
    if let Some(anchor) = config.anchor.as_ref() {
        assert_with_error!(
            env,
            !config.primary.reads_same_feed_as(anchor),
            GenericError::InvalidExchangeSrc
        );
    }

    // Single markets may use a spot primary: `validate_single_source_sanity_band`
    // (called separately by the composing validator) already caps their sanity
    // band to `MAX_SINGLE_SOURCE_SANITY_BAND_BPS`, which is the read-mode-independent
    // defense for a single unchecked source. TWAP smoothing is redundant on top of
    // that band and not required for slow-moving feeds (e.g. RWA NAV prices).
    #[cfg(not(feature = "testing"))]
    {
        let primary_is_spot = match &config.primary {
            OracleSourceConfigInput::Reflector(r) => matches!(r.read_mode, OracleReadMode::Spot),
            OracleSourceConfigInput::RedStone(_) | OracleSourceConfigInput::Xoxno(_) => true,
        };

        // Production anchored markets require a non-spot primary.
        if config.strategy == OracleStrategy::PrimaryWithAnchor {
            if primary_is_spot {
                panic_with_error!(env, GenericError::SpotOnlyNotProductionSafe);
            }
            if let Some(anchor) = config.anchor.as_ref() {
                // Anchor and primary must come from different oracle providers.
                let same_provider = matches!(
                    (&config.primary, anchor),
                    (
                        OracleSourceConfigInput::Reflector(_),
                        OracleSourceConfigInput::Reflector(_)
                    ) | (
                        OracleSourceConfigInput::RedStone(_),
                        OracleSourceConfigInput::RedStone(_)
                    ) | (
                        OracleSourceConfigInput::Xoxno(_),
                        OracleSourceConfigInput::Xoxno(_)
                    )
                );
                if same_provider {
                    panic_with_error!(env, GenericError::InvalidExchangeSrc);
                }
                // The dual-ABI XOXNO adapter must not back both legs.
                if config.primary.contract() == anchor.contract() {
                    panic_with_error!(env, GenericError::InvalidExchangeSrc);
                }
            }
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

pub(crate) fn validate_sanity_bounds(env: &Env, min_wad: i128, max_wad: i128) {
    common_validate_sanity_bounds(env, min_wad, max_wad);
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

#[cfg(test)]
#[path = "../../tests/validate/oracle_config.rs"]
mod tests;
