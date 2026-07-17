//! Reflector SEP-40 price provider: spot or TWAP read, repricing a quoted base
//! into USD. A TWAP that cannot be computed reverts; there is no spot fallback.

use common::errors::OracleError;
use common::math::fp::Wad;
use common::oracle::observation::{
    check_not_future_at, is_stale, normalize_positive_price, MAX_TWAP_RECORDS,
};
use common::oracle::providers::reflector::{
    min_twap_observations, reflector_lastprice_call, reflector_prices_call, to_reflector_asset,
    twap_mean_price,
};
use common::types::{
    OracleReadMode, OracleSourceConfig, PriceFeedRaw, ReflectorBase, ReflectorSourceConfig,
};
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env};

use crate::context::Cache;
use crate::oracle;
use crate::oracle::observation::{reflector_observation_from_price_data, OracleObservation};
use crate::storage;

pub(crate) fn read_reflector_source(
    cache: &mut Cache,
    config: &ReflectorSourceConfig,
    max_stale: u64,
) -> Option<OracleObservation> {
    let observation = match config.read_mode {
        OracleReadMode::Spot => read_spot(cache.env(), config),
        OracleReadMode::Twap(records) => Some(read_twap(cache, config, records, max_stale)),
    };
    observation.map(|obs| reprice_to_usd(cache, &config.base, obs))
}

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
                // Freshness is the staler of token and quote legs.
                observed_at: obs.observed_at.min(quote_feed.timestamp),
                published_at: obs.published_at,
            }
        }
    }
}

/// Resolves the USD price of a quote asset for repricing.
fn resolve_usd_quote(cache: &mut Cache, quote: &Address) -> PriceFeedRaw {
    let env = cache.env().clone();
    // Quote needs a token-rooted `AssetOracle` (base config, not spoke override).
    let Some(oracle_config) = storage::get_asset_oracle(&env, quote) else {
        panic_with_error!(&env, OracleError::InvalidOracleBase)
    };
    match &oracle_config.primary {
        OracleSourceConfig::RedStone(_) | OracleSourceConfig::Xoxno(_) => {}
        // Reflector quote primary must be USD (no chaining).
        OracleSourceConfig::Reflector(r) => match &r.base {
            ReflectorBase::Usd => {}
            _ => panic_with_error!(&env, OracleError::InvalidOracleBase),
        },
    }
    oracle::token_price(cache, quote)
}

/// Spot read via Reflector `lastprice`. `None` when the feed has no price.
fn read_spot(env: &Env, config: &ReflectorSourceConfig) -> Option<OracleObservation> {
    let asset = to_reflector_asset(env, &config.asset);
    let pd = reflector_lastprice_call(env, &config.contract, &asset)?;
    Some(reflector_observation_from_price_data(
        env,
        &pd,
        config.decimals,
    ))
}

/// TWAP over returned samples; reverts if history is missing/stale/invalid (no spot fallback).
fn read_twap(
    cache: &Cache,
    config: &ReflectorSourceConfig,
    records: u32,
    max_stale: u64,
) -> OracleObservation {
    let env = cache.env();
    if records == 0 {
        panic_with_error!(env, OracleError::TwapInsufficientObservations);
    }
    assert_with_error!(
        env,
        records <= MAX_TWAP_RECORDS,
        OracleError::InvalidOracleTokenType
    );

    let asset = to_reflector_asset(env, &config.asset);
    let Some(history) = reflector_prices_call(env, &config.contract, &asset, records) else {
        panic_with_error!(env, OracleError::ReflectorHistoryEmpty);
    };
    if history.is_empty() {
        panic_with_error!(env, OracleError::ReflectorHistoryEmpty);
    }

    let mut oldest_ts = u64::MAX;
    for pd in history.iter() {
        check_not_future_at(env, cache.ledger_timestamp_secs(), pd.timestamp);
        if pd.timestamp < oldest_ts {
            oldest_ts = pd.timestamp;
        }
    }

    if history.len() < min_twap_observations(records) {
        panic_with_error!(env, OracleError::TwapInsufficientObservations);
    }
    if is_stale(cache.ledger_timestamp_secs(), oldest_ts, max_stale) {
        panic_with_error!(env, OracleError::PriceFeedStale);
    }

    // Mean over returned samples (not requested count); shared with governance probe.
    let raw_price = twap_mean_price(env, &history);
    OracleObservation {
        price_wad: normalize_positive_price(env, raw_price, config.decimals),
        observed_at: oldest_ts,
        published_at: None,
    }
}
