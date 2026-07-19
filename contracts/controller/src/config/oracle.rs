//! Per-asset oracle configuration setters and validation. Oracles are
//! token-rooted (hub-independent) and must resolve to a USD base.

use common::errors::{GenericError, OracleError};
use common::types::{
    HubAssetKey, MarketOracleConfig, OraclePriceFluctuation, OracleSourceConfig, OracleStrategy,
    ReflectorBase,
};
use common::validation::{
    validate_oracle_tolerance, validate_sanity_bounds, validate_single_source_sanity_band,
};
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env};

use crate::context::Cache;
use crate::external::pool::fetch_pool_sync_data;
use crate::{
    events::{EventOracleProvider, UpdateAssetOracleEvent},
    storage,
};

/// Configures the token-rooted oracle for an existing market after revalidating
/// sanity bands and quote-market activity.
pub(crate) fn set_market_oracle_config(env: &Env, hub_asset: HubAssetKey, mut config: MarketOracleConfig) {
    let asset = &hub_asset.asset;
    // Only an existing `(hub, asset)` market can be activated. The pool owns the
    // market record; `fetch_pool_sync_data` reverts `PoolNotInitialized` when the
    // market was never created.
    let pool_addr = storage::get_pool(env);
    let pool_decimals = fetch_pool_sync_data(env, &pool_addr, &hub_asset)
        .params
        .asset_decimals;

    // Re-validate at execution (not only at propose).
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

    // Token-rooted (hub-independent), keyed by bare asset.
    storage::set_asset_oracle(env, asset, &config);

    UpdateAssetOracleEvent {
        asset: asset.clone(),
        oracle: EventOracleProvider::from_oracle(env, asset, &config),
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
    // (oracle::compose). Re-validate it here so neither a direct owner call nor
    // the spoke oracle-override path can store a degenerate band that disables
    // the agreement guard.
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
/// The bot incident path for band exits: the new band must contain the
/// current live price, proven by resolving the price under the new band
/// (bypassing the old one) — out-of-band reverts `SanityBoundViolated`,
/// a stale feed reverts `PriceFeedStale`. The new band must also overlap the
/// old one: a band can be walked (each step live-price-contained and
/// evented), never teleported to a disjoint range on one transient print.
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

    // Containment probe with the updated config; nothing is cached or stored
    // unless the live price sits inside the new band. Read-only: the
    // entrypoint already renewed the instance TTL.
    let mut cache = Cache::new_view(env);
    crate::oracle::price_with_config(&mut cache, &asset, &oracle);

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
    common::validation::validate_oracle_tolerance(env, &tolerance);
    oracle.tolerance = tolerance;
    storage::set_asset_oracle(env, &asset, &oracle);

    UpdateAssetOracleEvent {
        asset: asset.clone(),
        oracle: EventOracleProvider::from_oracle(env, &asset, &oracle),
    }
    .publish(env);
}
