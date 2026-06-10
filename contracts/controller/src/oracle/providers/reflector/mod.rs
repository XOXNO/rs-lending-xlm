// Reflector SEP-40 price provider.

pub(crate) mod client;
mod spot;
mod twap;

use common::errors::OracleError;
use common::math::fp::Wad;
use common::types::{
    MarketStatus, OracleAssetRef, OracleReadMode, OracleSourceConfig, PriceFeedRaw, ReflectorBase,
    ReflectorSourceConfig,
};
use soroban_sdk::{panic_with_error, Address, Env};

use super::super::observation::{build_observation, check_not_future_at, OracleObservation};
use crate::cache::Cache;

pub(crate) use client::{
    reflector_base_call, reflector_decimals_call, reflector_lastprice_call, reflector_prices_call,
    reflector_resolution_call, ReflectorAsset, ReflectorPriceData,
};

pub(crate) use twap::min_twap_observations;

pub(crate) fn to_reflector_asset(env: &Env, asset: &OracleAssetRef) -> ReflectorAsset {
    match asset {
        OracleAssetRef::Stellar(address) => ReflectorAsset::Stellar(address.clone()),
        OracleAssetRef::Symbol(symbol) => ReflectorAsset::Other(symbol.clone()),
        OracleAssetRef::String(_) => panic_with_error!(env, OracleError::InvalidOracleTokenType),
    }
}

pub(crate) fn read_reflector_source(
    cache: &mut Cache,
    config: &ReflectorSourceConfig,
    max_stale: u64,
    required: bool,
) -> Option<OracleObservation> {
    let observation = match config.read_mode {
        OracleReadMode::Spot => spot::read_spot(cache.env(), config, required),
        OracleReadMode::Twap(records) => {
            twap::read_twap(cache, config, records, max_stale, required)
        }
    };
    observation.map(|obs| reprice_to_usd(cache, &config.base, obs))
}

/// Converts a Reflector observation into USD WAD.
fn reprice_to_usd(
    cache: &mut Cache,
    base: &ReflectorBase,
    obs: OracleObservation,
) -> OracleObservation {
    match base {
        ReflectorBase::Usd => obs,
        ReflectorBase::Quoted(quote) => {
            let env = cache.env().clone();
            let quote_feed = resolve_usd_quote(cache, quote);
            let price_usd = Wad::from(obs.price_wad)
                .mul(&env, Wad::from(quote_feed.price_wad))
                .raw();
            OracleObservation {
                price_wad: price_usd,
                // The composite is only as fresh as its staler leg: bound the
                // token timestamp by the quote's so stale-tolerating policies
                // see the quote's age too.
                observed_at: obs.observed_at.min(quote_feed.timestamp),
                published_at: obs.published_at,
            }
        }
    }
}

/// Resolves the USD price of a quote asset for repricing.
fn resolve_usd_quote(cache: &mut Cache, quote: &Address) -> PriceFeedRaw {
    let env = cache.env().clone();
    let market = cache.cached_market_config(quote);
    if market.status != MarketStatus::Active {
        panic_with_error!(&env, OracleError::InvalidOracleBase);
    }
    match &market.oracle_config.primary {
        // RedStone feeds are USD-denominated by construction.
        OracleSourceConfig::RedStone(_) => {}
        // A Reflector quote source must itself be USD-based (no chaining).
        // Read the base cached at config time — no live `base()` call.
        OracleSourceConfig::Reflector(r) => match &r.base {
            ReflectorBase::Usd => {}
            _ => panic_with_error!(&env, OracleError::InvalidOracleBase),
        },
    }
    crate::oracle::token_price(cache, quote)
}

pub(crate) fn observation_from_price_data(
    env: &Env,
    pd: &ReflectorPriceData,
    decimals: u32,
) -> OracleObservation {
    check_not_future_at(env, env.ledger().timestamp(), pd.timestamp);
    build_observation(env, pd.price, decimals, pd.timestamp, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Covers the `OracleAssetRef::Symbol` mapping in `to_reflector_asset`.
    // The production harness only registers `Stellar(addr)` markets so this
    // variant has no organic exercise — direct unit test fills the gap.
    #[test]
    fn test_to_reflector_asset_symbol_maps_to_other() {
        let env = Env::default();
        let symbol = soroban_sdk::Symbol::new(&env, "USD");
        let asset = OracleAssetRef::Symbol(symbol.clone());
        let result = to_reflector_asset(&env, &asset);
        match result {
            ReflectorAsset::Other(s) => assert_eq!(s, symbol),
            _ => panic!("expected ReflectorAsset::Other"),
        }
    }

    // `OracleAssetRef::String` is unsupported on Reflector — must panic
    // with `InvalidOracleTokenType`.
    #[test]
    #[should_panic]
    fn test_to_reflector_asset_string_panics() {
        let env = Env::default();
        let asset = OracleAssetRef::String(soroban_sdk::String::from_str(&env, "USDC"));
        let _ = to_reflector_asset(&env, &asset);
    }
}
