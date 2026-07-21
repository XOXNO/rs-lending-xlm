//! Token-rooted oracle configuration setters and validation. Oracles are
//! hub-independent and must resolve to a USD base. Callers are owner-gated in
//! `lib.rs`; the resolved `MarketOracleConfig` (including `asset_decimals`,
//! sourced from the SAC token by the governance resolver) arrives pre-built.

use common::errors::{GenericError, OracleError};
use common::types::{
    MarketOracleConfig, OraclePriceFluctuation, OracleSourceConfig, OracleStrategy, ReflectorBase,
};
use common::validation::{
    validate_oracle_tolerance, validate_sanity_bounds, validate_single_source_sanity_band,
};
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env};

use crate::context::PriceCache;
use crate::events::{EventOracleProvider, UpdateAssetOracleEvent};
use crate::price::price_with_config;
use crate::storage;

/// Stores the token-rooted oracle config after revalidating sanity bands,
/// tolerance, and quote-market activity.
pub(crate) fn set_market_oracle_config(env: &Env, asset: Address, config: MarketOracleConfig) {
    validate_market_oracle_config(env, &asset, &config);
    storage::set_asset_oracle(env, &asset, &config);

    UpdateAssetOracleEvent {
        asset: asset.clone(),
        oracle: EventOracleProvider::from_oracle(env, &asset, &config),
    }
    .publish(env);
}

pub(crate) fn validate_market_oracle_config(
    env: &Env,
    asset: &Address,
    config: &MarketOracleConfig,
) {
    validate_sanity_bounds(
        env,
        config.min_sanity_price_wad,
        config.max_sanity_price_wad,
    );
    validate_single_source_sanity_band(
        env,
        config.strategy,
        config.min_sanity_price_wad,
        config.max_sanity_price_wad,
    );
    // The primary/anchor agreement band is consumed only for anchored configs
    // (compose). Re-validate it here so a degenerate band cannot disable the
    // agreement guard.
    if config.strategy == OracleStrategy::PrimaryWithAnchor {
        validate_oracle_tolerance(env, &config.tolerance);
    }
    require_quote_markets_active_usd(env, asset, config);
}

fn require_quote_markets_active_usd(env: &Env, asset: &Address, config: &MarketOracleConfig) {
    require_source_quote_active_usd(env, asset, &config.primary);
    if let Some(anchor) = config.anchor.as_ref() {
        require_source_quote_active_usd(env, asset, anchor);
    }
}

fn require_source_quote_active_usd(env: &Env, asset: &Address, source: &OracleSourceConfig) {
    let OracleSourceConfig::Reflector(reflector) = source else {
        return;
    };
    let ReflectorBase::Quoted(quote) = &reflector.base else {
        return;
    };

    // Reject self-quotes at config time (not only via the read-time cycle guard).
    assert_with_error!(env, quote != asset, OracleError::InvalidOracleBase);

    // Quote needs an active token-rooted `AssetOracle`.
    let Some(quote_oracle) = storage::get_asset_oracle(env, quote) else {
        panic_with_error!(env, OracleError::InvalidOracleBase)
    };

    // Quote primary must be USD (one hop, no quote chains).
    match &quote_oracle.primary {
        OracleSourceConfig::RedStone(_) | OracleSourceConfig::Xoxno(_) => {}
        OracleSourceConfig::Reflector(quote_primary) => assert_with_error!(
            env,
            matches!(quote_primary.base, ReflectorBase::Usd),
            OracleError::InvalidOracleBase
        ),
    }
}

/// Moves only the sanity band on an active oracle, keeping every other field.
/// The new band must contain the current live price (proven by resolving it
/// under the new band) and overlap the old one: a band can be walked, never
/// teleported to a disjoint range on one transient print.
pub(crate) fn set_oracle_sanity_bounds(env: &Env, asset: Address, min_wad: i128, max_wad: i128) {
    let mut oracle = storage::get_asset_oracle(env, &asset)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PairNotActive));

    validate_sanity_bounds(env, min_wad, max_wad);
    validate_single_source_sanity_band(env, oracle.strategy, min_wad, max_wad);
    assert_with_error!(
        env,
        min_wad < oracle.max_sanity_price_wad && max_wad > oracle.min_sanity_price_wad,
        OracleError::InvalidSanityBounds
    );
    oracle.min_sanity_price_wad = min_wad;
    oracle.max_sanity_price_wad = max_wad;

    // Containment probe with the updated config; nothing is stored unless the
    // live price sits inside the new band.
    let mut cache = PriceCache::new_view(env);
    price_with_config(&mut cache, &asset, &oracle);

    storage::set_asset_oracle(env, &asset, &oracle);

    UpdateAssetOracleEvent {
        asset: asset.clone(),
        oracle: EventOracleProvider::from_oracle(env, &asset, &oracle),
    }
    .publish(env);
}

pub(crate) fn set_oracle_tolerance(env: &Env, asset: Address, tolerance: OraclePriceFluctuation) {
    let mut oracle = storage::get_asset_oracle(env, &asset)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PairNotActive));
    // Re-validate at execution so a direct owner call cannot store a bad band.
    validate_oracle_tolerance(env, &tolerance);
    oracle.tolerance = tolerance;
    storage::set_asset_oracle(env, &asset, &oracle);

    UpdateAssetOracleEvent {
        asset: asset.clone(),
        oracle: EventOracleProvider::from_oracle(env, &asset, &oracle),
    }
    .publish(env);
}
