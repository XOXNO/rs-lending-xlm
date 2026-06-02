//! Pure configuration and shape validation for oracle sources.
//!
//! Checks here never make live calls to external oracle contracts
//! (Reflector or RedStone), so they are safe to run in any context.

use common::errors::{GenericError, OracleError};
use common::types::{MarketOracleConfigInput, OracleSourceConfigInput, OracleStrategy};
#[cfg(not(feature = "testing"))]
use common::types::OracleReadMode;
use soroban_sdk::{assert_with_error, panic_with_error, Env};

use super::super::observation::{
    MAX_ORACLE_DECIMALS, MAX_PRICE_STALE_SECONDS, MAX_TWAP_RECORDS, MIN_ORACLE_DECIMALS,
    MIN_PRICE_STALE_SECONDS,
};

/// Validates the structural shape of a market oracle config (no live calls).
/// Enforces, in order:
/// 1. strategy/anchor coherence — `PrimaryWithAnchor` iff an anchor is set;
/// 2. feed diversity — primary and anchor must not read the same feed
///    (`sources_read_same_feed`), ignoring policy-only fields;
/// 3. production only (`#[cfg(not(feature = "testing"))]`):
///    - a `Single` market's primary may not be naked spot (a Reflector `Spot`
///      read or any RedStone source) → `SpotOnlyNotProductionSafe`;
///    - a `PrimaryWithAnchor` pair must cross providers (one Reflector, one
///      RedStone) → `InvalidExchangeSrc`.
///
/// Rules 1–2 run in every build; rule 3 is relaxed under `testing` so unit
/// tests can use simplified single-provider configs.
pub(crate) fn validate_oracle_config_shape(env: &Env, config: &MarketOracleConfigInput) {
    let needs_anchor = config.strategy == OracleStrategy::PrimaryWithAnchor;
    let has_anchor = !config.anchor.is_none();
    assert_with_error!(
        env,
        needs_anchor == has_anchor,
        GenericError::InvalidExchangeSrc
    );

    // Primary and anchor must read different underlying feeds: two sources on
    // the same feed collapse the dual check into one, so `is_within_anchor`
    // compares a price against itself (~1.0), always passes the tolerance band,
    // and voids the diversity guarantee. Feed identity ignores policy-only
    // fields (RedStone `max_stale_seconds`), so the same RedStone contract+feed
    // cannot be paired with itself under a different staleness bound.
    if let Some(anchor) = config.anchor.as_ref() {
        assert_with_error!(
            env,
            !sources_read_same_feed(&config.primary, anchor),
            GenericError::InvalidExchangeSrc
        );
    }

    // A Single-strategy market has no anchor cross-check, so in production its
    // primary must carry temporal diversity: only a Reflector TWAP is allowed.
    // A Reflector Spot or any RedStone primary (RedStone always reads spot) is
    // naked spot and rejected — use PrimaryWithAnchor instead (INVARIANTS §4.3,
    // ADR-0003).
    #[cfg(not(feature = "testing"))]
    {
        let primary_is_spot = match &config.primary {
            OracleSourceConfigInput::Reflector(r) => matches!(r.read_mode, OracleReadMode::Spot),
            OracleSourceConfigInput::RedStone(_) => true,
        };
        if config.strategy == OracleStrategy::Single && primary_is_spot {
            panic_with_error!(env, GenericError::SpotOnlyNotProductionSafe);
        }

        // A PrimaryWithAnchor market must cross provider trust boundaries: the
        // primary and anchor must come from different providers (one Reflector,
        // one RedStone). A single provider's failure — bad feed, signer or
        // contract compromise, feed-mapping error — then moves only one side, so
        // the deviation check catches it instead of both sources sliding
        // together past the band.
        if config.strategy == OracleStrategy::PrimaryWithAnchor {
            if let Some(anchor) = config.anchor.as_ref() {
                let same_provider = matches!(
                    (&config.primary, anchor),
                    (
                        OracleSourceConfigInput::Reflector(_),
                        OracleSourceConfigInput::Reflector(_)
                    ) | (
                        OracleSourceConfigInput::RedStone(_),
                        OracleSourceConfigInput::RedStone(_)
                    )
                );
                if same_provider {
                    panic_with_error!(env, GenericError::InvalidExchangeSrc);
                }
            }
        }
    }
}

/// True when two source configs read the same underlying feed. Compares feed
/// identity only — provider plus contract and feed key (Reflector: asset and
/// read mode; RedStone: feed id) — and ignores policy-only fields such as
/// RedStone `max_stale_seconds`. Cross-provider sources are always distinct.
fn sources_read_same_feed(a: &OracleSourceConfigInput, b: &OracleSourceConfigInput) -> bool {
    match (a, b) {
        (OracleSourceConfigInput::Reflector(x), OracleSourceConfigInput::Reflector(y)) => {
            x.contract == y.contract && x.asset == y.asset && x.read_mode == y.read_mode
        }
        (OracleSourceConfigInput::RedStone(x), OracleSourceConfigInput::RedStone(y)) => {
            x.contract == y.contract && x.feed_id == y.feed_id
        }
        _ => false,
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
