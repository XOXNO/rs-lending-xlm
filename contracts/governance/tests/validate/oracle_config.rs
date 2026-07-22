use super::*;
use common::constants::MAX_REASONABLE_PRICE_WAD;
#[cfg(not(feature = "testing"))]
use common::types::RedStoneSourceConfigInput;
use common::types::{
    AssetOracleConfigInput, OracleAssetRef, OracleReadMode, OracleSourceConfigInput,
    OracleSourceConfigInputOption, OracleStrategy, ReflectorSourceConfigInput,
};
use soroban_sdk::testutils::Address as _;
#[cfg(not(feature = "testing"))]
use soroban_sdk::String;
use soroban_sdk::{Address, Env};

fn sample_config(
    strategy: OracleStrategy,
    primary: OracleSourceConfigInput,
) -> AssetOracleConfigInput {
    AssetOracleConfigInput {
        max_price_stale_seconds: 900,
        tolerance_bps: 500,
        min_sanity_price_wad: 1,
        max_sanity_price_wad: MAX_REASONABLE_PRICE_WAD,
        strategy,
        primary,
        anchor: OracleSourceConfigInputOption::None,
    }
}

fn reflector_source(
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
        OracleStrategy::Single,
        reflector_source(&contract, &asset, OracleReadMode::Twap(5)),
    );
    validate_oracle_config_shape(&env, &cfg);
}

#[test]
#[cfg(not(feature = "testing"))]
fn test_single_spot_primary_passes_production_shape() {
    let env = Env::default();
    let contract = Address::generate(&env);
    let asset = OracleAssetRef::Stellar(Address::generate(&env));
    let cfg = sample_config(
        OracleStrategy::Single,
        reflector_source(&contract, &asset, OracleReadMode::Spot),
    );
    validate_oracle_config_shape(&env, &cfg);
}

#[test]
#[cfg(not(feature = "testing"))]
fn test_single_redstone_primary_passes_production_shape() {
    let env = Env::default();
    let primary = OracleSourceConfigInput::RedStone(RedStoneSourceConfigInput {
        contract: Address::generate(&env),
        feed_id: String::from_str(&env, "ETH/USD"),
        max_stale_seconds: 900,
    });
    let cfg = sample_config(OracleStrategy::Single, primary);
    validate_oracle_config_shape(&env, &cfg);
}

#[test]
#[cfg(not(feature = "testing"))]
#[should_panic(expected = "Error(Contract, #11)")]
fn test_dual_same_reflector_provider_rejects() {
    let env = Env::default();
    let contract = Address::generate(&env);
    let asset = OracleAssetRef::Stellar(Address::generate(&env));
    let mut cfg = sample_config(
        OracleStrategy::PrimaryWithAnchor,
        reflector_source(&contract, &asset, OracleReadMode::Twap(5)),
    );
    cfg.anchor = OracleSourceConfigInputOption::Some(reflector_source(
        &contract,
        &asset,
        OracleReadMode::Spot,
    ));
    validate_oracle_config_shape(&env, &cfg);
}

#[test]
#[cfg(not(feature = "testing"))]
#[should_panic(expected = "Error(Contract, #38)")]
fn test_dual_redstone_primary_rejects_spot_only_mode() {
    let env = Env::default();
    let contract = Address::generate(&env);
    let feed_a = String::from_str(&env, "BTC/USD");
    let feed_b = String::from_str(&env, "ETH/USD");
    let primary = OracleSourceConfigInput::RedStone(RedStoneSourceConfigInput {
        contract: contract.clone(),
        feed_id: feed_a,
        max_stale_seconds: 600,
    });
    let mut cfg = sample_config(OracleStrategy::PrimaryWithAnchor, primary);
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
#[cfg(not(feature = "testing"))]
fn test_single_xoxno_primary_passes_production_shape() {
    let env = Env::default();
    let primary = OracleSourceConfigInput::Xoxno(RedStoneSourceConfigInput {
        contract: Address::generate(&env),
        feed_id: String::from_str(&env, "ETH/USD"),
        max_stale_seconds: 900,
    });
    let cfg = sample_config(OracleStrategy::Single, primary);
    validate_oracle_config_shape(&env, &cfg);
}

#[test]
#[cfg(not(feature = "testing"))]
fn test_dual_reflector_twap_primary_with_xoxno_anchor_passes() {
    let env = Env::default();
    let contract = Address::generate(&env);
    let asset = OracleAssetRef::Stellar(Address::generate(&env));
    let mut cfg = sample_config(
        OracleStrategy::PrimaryWithAnchor,
        reflector_source(&contract, &asset, OracleReadMode::Twap(5)),
    );
    cfg.anchor = OracleSourceConfigInputOption::Some(OracleSourceConfigInput::Xoxno(
        RedStoneSourceConfigInput {
            contract: Address::generate(&env),
            feed_id: String::from_str(&env, "ETH/USD"),
            max_stale_seconds: 900,
        },
    ));
    validate_oracle_config_shape(&env, &cfg);
}

#[test]
#[cfg(not(feature = "testing"))]
#[should_panic(expected = "Error(Contract, #38)")]
fn test_dual_xoxno_primary_rejects_spot_only_mode() {
    let env = Env::default();
    let primary = OracleSourceConfigInput::Xoxno(RedStoneSourceConfigInput {
        contract: Address::generate(&env),
        feed_id: String::from_str(&env, "BTC/USD"),
        max_stale_seconds: 600,
    });
    let mut cfg = sample_config(OracleStrategy::PrimaryWithAnchor, primary);
    cfg.anchor = OracleSourceConfigInputOption::Some(OracleSourceConfigInput::Xoxno(
        RedStoneSourceConfigInput {
            contract: Address::generate(&env),
            feed_id: String::from_str(&env, "ETH/USD"),
            max_stale_seconds: 900,
        },
    ));
    validate_oracle_config_shape(&env, &cfg);
}

#[test]
#[cfg(not(feature = "testing"))]
#[should_panic(expected = "Error(Contract, #11)")]
fn test_dual_shared_contract_across_variants_rejects() {
    // The dual-ABI adapter listed once as SEP-40 primary and once as a
    // Xoxno anchor is one aggregation state, not two opinions.
    let env = Env::default();
    let contract = Address::generate(&env);
    let asset = OracleAssetRef::Stellar(Address::generate(&env));
    let mut cfg = sample_config(
        OracleStrategy::PrimaryWithAnchor,
        reflector_source(&contract, &asset, OracleReadMode::Twap(5)),
    );
    cfg.anchor = OracleSourceConfigInputOption::Some(OracleSourceConfigInput::Xoxno(
        RedStoneSourceConfigInput {
            contract,
            feed_id: String::from_str(&env, "ETH/USD"),
            max_stale_seconds: 900,
        },
    ));
    validate_oracle_config_shape(&env, &cfg);
}

#[test]
#[should_panic(expected = "Error(Contract, #11)")]
fn test_dual_redstone_and_xoxno_same_feed_rejects() {
    // Same (contract, feed_id) wire feed under two variant names is still the
    // same feed; `reads_same_feed_as` blocks it in every build.
    let env = Env::default();
    let contract = Address::generate(&env);
    let feed_id = soroban_sdk::String::from_str(&env, "BTC/USD");
    let primary = OracleSourceConfigInput::RedStone(common::types::RedStoneSourceConfigInput {
        contract: contract.clone(),
        feed_id: feed_id.clone(),
        max_stale_seconds: 600,
    });
    let mut cfg = sample_config(OracleStrategy::PrimaryWithAnchor, primary);
    cfg.anchor = OracleSourceConfigInputOption::Some(OracleSourceConfigInput::Xoxno(
        common::types::RedStoneSourceConfigInput {
            contract,
            feed_id,
            max_stale_seconds: 900,
        },
    ));
    validate_oracle_config_shape(&env, &cfg);
}

#[test]
#[should_panic(expected = "Error(Contract, #224)")]
fn test_validate_sanity_bounds_rejects_non_positive() {
    let env = Env::default();
    validate_sanity_bounds(&env, 0, MAX_REASONABLE_PRICE_WAD);
}

#[test]
#[should_panic(expected = "Error(Contract, #224)")]
fn test_validate_sanity_bounds_rejects_min_ge_max() {
    let env = Env::default();
    validate_sanity_bounds(&env, 100, 100);
}

#[test]
#[should_panic(expected = "Error(Contract, #224)")]
fn test_validate_sanity_bounds_rejects_max_above_cap() {
    let env = Env::default();
    validate_sanity_bounds(&env, 1, MAX_REASONABLE_PRICE_WAD + 1);
}

#[test]
#[should_panic(expected = "Error(Contract, #218)")]
fn test_validate_max_stale_rejects_below_floor() {
    let env = Env::default();
    validate_max_stale(&env, MIN_PRICE_STALE_SECONDS - 1);
}

#[test]
#[should_panic(expected = "Error(Contract, #221)")]
fn test_validate_decimals_rejects_out_of_range() {
    let env = Env::default();
    validate_decimals(&env, MIN_ORACLE_DECIMALS - 1);
}

#[test]
#[should_panic(expected = "Error(Contract, #219)")]
fn test_validate_twap_records_rejects_zero() {
    let env = Env::default();
    validate_twap_records(&env, 0);
}
