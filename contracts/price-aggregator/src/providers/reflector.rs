//! Reflector SEP-40 price provider: spot or TWAP read, repricing a quoted base
//! into USD. A TWAP that cannot be computed reverts; there is no spot fallback.

use common::errors::OracleError;
use common::math::fp::Wad;
use common::oracle::observation::{check_not_future_at, is_stale, normalize_positive_price};
use common::oracle::providers::reflector::{
    min_twap_observations, reflector_lastprice_call, reflector_prices_call, to_reflector_asset,
    twap_mean_price,
};
use common::types::{OracleReadMode, PriceFeedRaw, ReflectorBase, ReflectorSourceConfig};
use common::validation::validate_twap_records;
use soroban_sdk::{panic_with_error, Address};

use crate::config::require_usd_rooted;
use crate::context::ResolutionContext;
use crate::observation::OracleObservation;
use crate::price;

pub(crate) fn read_reflector_source(
    cache: &mut ResolutionContext,
    config: &ReflectorSourceConfig,
    max_stale: u64,
) -> Option<OracleObservation> {
    let observation = match config.read_mode {
        OracleReadMode::Spot => read_spot(cache, config),
        OracleReadMode::Twap(records) => Some(read_twap(cache, config, records, max_stale)),
    };
    observation.map(|obs| reprice_to_usd(cache, &config.base, obs))
}

fn reprice_to_usd(
    cache: &mut ResolutionContext,
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

/// Resolves the USD price of a quote asset for repricing. Read-time backstop
/// of the config-time rule: the quote needs its own USD-rooted `AssetOracle`.
fn resolve_usd_quote(cache: &mut ResolutionContext, quote: &Address) -> PriceFeedRaw {
    let env = cache.env().clone();
    let Some(quote_oracle) = cache.cached_asset_oracle_opt(quote) else {
        panic_with_error!(&env, OracleError::InvalidOracleBase)
    };
    require_usd_rooted(&env, &quote_oracle);
    price::resolve_usd_price(cache, quote)
}

/// Spot read via Reflector `lastprice`. `None` when the feed has no price.
fn read_spot(
    cache: &ResolutionContext,
    config: &ReflectorSourceConfig,
) -> Option<OracleObservation> {
    let env = cache.env();
    let asset = to_reflector_asset(env, &config.asset);
    let price_data = reflector_lastprice_call(env, &config.contract, &asset)?;
    Some(OracleObservation::from_reflector(
        env,
        cache.ledger_timestamp_secs(),
        &price_data,
        config.decimals,
    ))
}

/// TWAP over returned samples; reverts if history is missing/stale/invalid (no spot fallback).
fn read_twap(
    cache: &ResolutionContext,
    config: &ReflectorSourceConfig,
    records: u32,
    max_stale: u64,
) -> OracleObservation {
    let env = cache.env();
    let now_secs = cache.ledger_timestamp_secs();
    validate_twap_records(env, records);

    let asset = to_reflector_asset(env, &config.asset);
    let Some(history) = reflector_prices_call(env, &config.contract, &asset, records) else {
        panic_with_error!(env, OracleError::ReflectorHistoryEmpty);
    };
    if history.is_empty() {
        panic_with_error!(env, OracleError::ReflectorHistoryEmpty);
    }
    if history.len() < min_twap_observations(records) {
        panic_with_error!(env, OracleError::TwapInsufficientObservations);
    }

    let mut oldest_ts = u64::MAX;
    for price_data in history.iter() {
        check_not_future_at(env, now_secs, price_data.timestamp);
        if price_data.timestamp < oldest_ts {
            oldest_ts = price_data.timestamp;
        }
    }
    if is_stale(now_secs, oldest_ts, max_stale) {
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
