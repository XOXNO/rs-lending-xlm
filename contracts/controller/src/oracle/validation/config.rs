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

/// Validates the structural shape of a market oracle config (no live calls).
/// Enforces, in order:
/// 1. strategy/anchor coherence — `PrimaryWithAnchor` iff an anchor is set;
/// 2. feed diversity — primary and anchor must not read the same feed
///    ([`OracleSourceConfigInput::reads_same_feed_as`]), ignoring policy-only fields;
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
            !config.primary.reads_same_feed_as(anchor),
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

#[cfg(test)]
mod tests {
    use super::*;
    use common::constants::MAX_REASONABLE_PRICE_WAD;
    use common::types::{
        MarketOracleConfigInput, OracleAssetRef, OracleReadMode, OracleSourceConfigInput,
        OracleSourceConfigInputOption, OracleStrategy, ReflectorSourceConfigInput,
        RedStoneSourceConfigInput,
    };
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env, String};

    fn sample_config(env: &Env, strategy: OracleStrategy, primary: OracleSourceConfigInput) -> MarketOracleConfigInput {
        MarketOracleConfigInput {
            max_price_stale_seconds: 900,
            first_tolerance_bps: 200,
            last_tolerance_bps: 500,
            min_sanity_price_wad: 1,
            max_sanity_price_wad: MAX_REASONABLE_PRICE_WAD,
            strategy,
            primary,
            anchor: OracleSourceConfigInputOption::None,
        }
    }

    fn reflector_source(
        env: &Env,
        contract: &Address,
        asset: &OracleAssetRef,
        read_mode: OracleReadMode,
    ) -> OracleSourceConfigInput {
        OracleSourceConfigInput::Reflector(ReflectorSourceConfigInput {
            contract: contract.clone(),
            asset: asset.clone(),
            read_mode,
        })
    }

    #[test]
    fn test_single_twap_primary_passes_production_shape() {
        let env = Env::default();
        let contract = Address::generate(&env);
        let asset = OracleAssetRef::Stellar(Address::generate(&env));
        let cfg = sample_config(
            &env,
            OracleStrategy::Single,
            reflector_source(&env, &contract, &asset, OracleReadMode::Twap(5)),
        );
        validate_oracle_config_shape(&env, &cfg);
    }

    #[test]
    #[should_panic]
    fn test_single_spot_primary_rejects_spot_only_production() {
        let env = Env::default();
        let contract = Address::generate(&env);
        let asset = OracleAssetRef::Stellar(Address::generate(&env));
        let cfg = sample_config(
            &env,
            OracleStrategy::Single,
            reflector_source(&env, &contract, &asset, OracleReadMode::Spot),
        );
        validate_oracle_config_shape(&env, &cfg);
    }

    #[test]
    #[should_panic]
    fn test_single_redstone_primary_rejects_spot_only_production() {
        let env = Env::default();
        let primary = OracleSourceConfigInput::RedStone(RedStoneSourceConfigInput {
            contract: Address::generate(&env),
            feed_id: String::from_str(&env, "ETH/USD"),
            max_stale_seconds: 900,
        });
        let cfg = sample_config(&env, OracleStrategy::Single, primary);
        validate_oracle_config_shape(&env, &cfg);
    }

    #[test]
    #[should_panic]
    fn test_dual_same_reflector_provider_rejects() {
        let env = Env::default();
        let contract = Address::generate(&env);
        let asset = OracleAssetRef::Stellar(Address::generate(&env));
        let mut cfg = sample_config(
            &env,
            OracleStrategy::PrimaryWithAnchor,
            reflector_source(&env, &contract, &asset, OracleReadMode::Twap(5)),
        );
        cfg.anchor = OracleSourceConfigInputOption::Some(reflector_source(
            &env,
            &contract,
            &asset,
            OracleReadMode::Spot,
        ));
        validate_oracle_config_shape(&env, &cfg);
    }

    #[test]
    #[should_panic]
    fn test_dual_same_redstone_provider_rejects() {
        let env = Env::default();
        let contract = Address::generate(&env);
        let feed_a = String::from_str(&env, "BTC/USD");
        let feed_b = String::from_str(&env, "ETH/USD");
        let primary = OracleSourceConfigInput::RedStone(RedStoneSourceConfigInput {
            contract: contract.clone(),
            feed_id: feed_a,
            max_stale_seconds: 600,
        });
        let mut cfg = sample_config(&env, OracleStrategy::PrimaryWithAnchor, primary);
        cfg.anchor = OracleSourceConfigInputOption::Some(OracleSourceConfigInput::RedStone(
            RedStoneSourceConfigInput {
                contract,
                feed_id: feed_b,
                max_stale_seconds: 900,
            },
        ));
        validate_oracle_config_shape(&env, &cfg);
    }

    #[test]
    #[should_panic]
    fn test_validate_sanity_bounds_rejects_non_positive() {
        let env = Env::default();
        validate_sanity_bounds(&env, 0, MAX_REASONABLE_PRICE_WAD);
    }

    #[test]
    #[should_panic]
    fn test_validate_sanity_bounds_rejects_min_ge_max() {
        let env = Env::default();
        validate_sanity_bounds(&env, 100, 100);
    }

    #[test]
    #[should_panic]
    fn test_validate_sanity_bounds_rejects_max_above_cap() {
        let env = Env::default();
        validate_sanity_bounds(&env, 1, MAX_REASONABLE_PRICE_WAD + 1);
    }

    #[test]
    #[should_panic]
    fn test_validate_max_stale_rejects_below_floor() {
        let env = Env::default();
        validate_max_stale(&env, MIN_PRICE_STALE_SECONDS - 1);
    }

    #[test]
    #[should_panic]
    fn test_validate_decimals_rejects_out_of_range() {
        let env = Env::default();
        validate_decimals(&env, MIN_ORACLE_DECIMALS - 1);
    }

    #[test]
    #[should_panic]
    fn test_validate_twap_records_rejects_zero() {
        let env = Env::default();
        validate_twap_records(&env, 0);
    }
}
