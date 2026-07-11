//! Per-asset oracle configuration setters and validation. Oracles are
//! token-rooted (hub-independent) and must resolve to a USD base.

use common::errors::{GenericError, OracleError};
use common::types::{
    HubAssetKey, MarketOracleConfig, OraclePriceFluctuation, OracleSourceConfig, ReflectorBase,
};
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env};

use crate::external::pool::fetch_pool_sync_data;
use crate::{
    events::{EventOracleProvider, UpdateAssetOracleEvent},
    storage,
};

/// Configures the token-rooted oracle for an existing market after revalidating
/// sanity bands and quote-market activity.
pub fn set_market_oracle_config(env: &Env, hub_asset: HubAssetKey, mut config: MarketOracleConfig) {
    let asset = &hub_asset.asset;
    // Only an existing `(hub, asset)` market can be activated. The pool owns the
    // market record; `fetch_pool_sync_data` reverts `PoolNotInitialized` when the
    // market was never created.
    let pool_addr = storage::get_pool(env);
    let pool_decimals = fetch_pool_sync_data(env, &pool_addr, &hub_asset)
        .params
        .asset_decimals;

    // Revalidates sanity bands and quote-market activity at execution.
    validate_market_oracle_config(env, asset, &config);

    // With `testing` enabled, preserve pool-registered decimals instead of a
    // live token probe.
    if cfg!(feature = "testing") && pool_decimals != 0 {
        config.asset_decimals = pool_decimals;
    }

    // Oracle decimals feed valuations and spoke-cap conversion; a mismatch
    // against the pool market mis-scales both by powers of ten.
    assert_with_error!(
        env,
        config.asset_decimals == pool_decimals,
        GenericError::InvalidAsset
    );

    // The oracle is token-rooted (hub-independent), keyed by the bare asset.
    storage::set_asset_oracle(env, asset, &config);

    UpdateAssetOracleEvent {
        asset: asset.clone(),
        oracle: EventOracleProvider::from_oracle(env, asset, &config),
    }
    .publish(env);
}

/// Validates market oracle config before storage.
pub(super) fn validate_market_oracle_config(
    env: &Env,
    asset: &Address,
    config: &MarketOracleConfig,
) {
    common::validation::validate_sanity_bounds(
        env,
        config.min_sanity_price_wad,
        config.max_sanity_price_wad,
    );
    common::validation::validate_single_source_sanity_band(
        env,
        config.strategy,
        config.min_sanity_price_wad,
        config.max_sanity_price_wad,
    );
    require_quote_markets_active_usd(env, asset, config);
}

/// Checks that quote sources point at active USD-based markets.
/// Direct USD and Other sources pass without lookup.
fn require_quote_markets_active_usd(env: &Env, asset: &Address, config: &MarketOracleConfig) {
    require_source_quote_active_usd(env, asset, &config.primary);
    if let Some(anchor) = config.anchor.as_ref() {
        require_source_quote_active_usd(env, asset, anchor);
    }
}

/// Requires a Reflector source's quoted base to be a distinct, active, one-hop USD asset.
fn require_source_quote_active_usd(env: &Env, asset: &Address, source: &OracleSourceConfig) {
    let OracleSourceConfig::Reflector(reflector) = source else {
        return;
    };
    let ReflectorBase::Quoted(quote) = &reflector.base else {
        return;
    };

    // A self-quote would otherwise only surface at read time, as a generic
    // `OracleCycleDetected` revert from the resolution-stack guard; reject it
    // here at config time with a more specific error.
    assert_with_error!(env, quote != asset, OracleError::InvalidOracleBase);

    // The quote must be active: a token-rooted `AssetOracle` entry must exist.
    let quote_oracle = match storage::get_asset_oracle(env, quote) {
        Some(oracle) => oracle,
        None => panic_with_error!(env, OracleError::InvalidOracleBase),
    };

    // The quote's primary must itself be USD-based: keeps the conversion exactly
    // one hop, forbidding a quote chain.
    match &quote_oracle.primary {
        OracleSourceConfig::RedStone(_) | OracleSourceConfig::Xoxno(_) => {}
        OracleSourceConfig::Reflector(quote_primary) => assert_with_error!(
            env,
            matches!(quote_primary.base, ReflectorBase::Usd),
            OracleError::InvalidOracleBase
        ),
    }
}

/// Updates the price-fluctuation tolerance on an active asset oracle.
pub fn set_oracle_tolerance(env: &Env, asset: Address, tolerance: OraclePriceFluctuation) {
    let mut oracle = storage::get_asset_oracle(env, &asset)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PairNotActive));
    oracle.tolerance = tolerance;
    storage::set_asset_oracle(env, &asset, &oracle);

    UpdateAssetOracleEvent {
        asset: asset.clone(),
        oracle: EventOracleProvider::from_oracle(env, &asset, &oracle),
    }
    .publish(env);
}
