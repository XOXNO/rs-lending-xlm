use common::errors::{GenericError, OracleError};
use common::types::{
    HubAssetKey, MarketOracleConfig, OraclePriceFluctuation, OracleSourceConfig, ReflectorBase,
};
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env};

use crate::external::pool::fetch_pool_sync_data;
use crate::{
    events::{EventOracleProvider, OracleDisabledEvent, UpdateAssetOracleEvent},
    storage,
};

pub fn set_market_oracle_config(env: &Env, hub_asset: HubAssetKey, mut config: MarketOracleConfig) {
    let asset = &hub_asset.asset;
    // Only an existing `(hub, asset)` market can be activated. The pool owns the
    // market record; `fetch_pool_sync_data` reverts `PoolNotInitialized` when the
    // market was never created.
    let pool_addr = storage::get_pool(env);
    let pool_decimals = fetch_pool_sync_data(env, &pool_addr, &hub_asset)
        .params
        .asset_decimals;

    // Re-validate the sanity band and quote-market USD/active invariant at the
    // controller boundary. Governance validates the proposal; execution rejects
    // unset or invalid bands, and timelock delay can make a quote market stale.
    validate_market_oracle_config(env, asset, &config);

    // Test markets register pools with preset decimals that may diverge from
    // the live token probe; keep the pool-registered value authoritative.
    if cfg!(feature = "testing") && pool_decimals != 0 {
        config.asset_decimals = pool_decimals;
    }

    // The oracle is token-rooted (hub-independent), keyed by the bare asset.
    storage::set_asset_oracle(env, asset, &config);

    UpdateAssetOracleEvent {
        asset: asset.clone(),
        oracle: EventOracleProvider::from_oracle(env, asset, &config),
    }
    .publish(env);
}

/// Validates a resolved `MarketOracleConfig` at the controller boundary: the
/// sanity band must be set and ordered, and every quote source must point at an
/// active, USD-based market. Shared by the token-rooted `set_market_oracle_config`
/// and the per-spoke `oracle_override`.
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

fn require_source_quote_active_usd(env: &Env, asset: &Address, source: &OracleSourceConfig) {
    let OracleSourceConfig::Reflector(reflector) = source else {
        return;
    };
    let ReflectorBase::Quoted(quote) = &reflector.base else {
        return;
    };

    // A market quoted in itself would chain forever at read time; reject it here.
    assert_with_error!(env, quote != asset, OracleError::InvalidOracleBase);

    // The quote must be active: a token-rooted `AssetOracle` entry must exist.
    let quote_oracle = match storage::get_asset_oracle(env, quote) {
        Some(oracle) => oracle,
        None => panic_with_error!(env, OracleError::InvalidOracleBase),
    };

    // The quote's primary must itself be USD-based: keeps the conversion exactly
    // one hop, forbidding a quote chain.
    match &quote_oracle.primary {
        OracleSourceConfig::RedStone(_) => {}
        OracleSourceConfig::Reflector(quote_primary) => assert_with_error!(
            env,
            matches!(quote_primary.base, ReflectorBase::Usd),
            OracleError::InvalidOracleBase
        ),
    }
}

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

/// Disables an active asset by removing its `AssetOracle` entry. Absence is the
/// disabled signal: price resolution then reverts for the asset.
pub fn disable_token_oracle(env: &Env, asset: Address) {
    assert_with_error!(
        env,
        storage::get_asset_oracle(env, &asset).is_some(),
        GenericError::PairNotActive
    );
    storage::remove_asset_oracle(env, &asset);
    OracleDisabledEvent { asset }.publish(env);
}
