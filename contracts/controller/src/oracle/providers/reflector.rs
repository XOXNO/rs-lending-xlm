//! Reflector SEP-40 price provider: spot or TWAP read, repricing a quoted base
//! into USD. A TWAP that cannot be computed reverts; there is no spot fallback.

use common::errors::{GenericError, OracleError};
use common::math::fp::Wad;
use common::oracle::observation::{
    check_not_future_at, is_stale, normalize_positive_price, MAX_TWAP_RECORDS,
};
use common::oracle::providers::reflector::{
    min_twap_observations, reflector_lastprice_call, reflector_prices_call, to_reflector_asset,
};
use common::types::{
    OracleReadMode, OracleSourceConfig, PriceFeedRaw, ReflectorBase, ReflectorSourceConfig,
};
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env};

use crate::context::Cache;
use crate::oracle;
use crate::oracle::observation::{reflector_observation_from_price_data, OracleObservation};
use crate::storage;

/// Reads a Reflector spot or TWAP observation and reprices it to USD.
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

/// Converts a quoted-base observation into USD, bounding freshness by the staler leg.
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
    // The quote must be active: a token-rooted `AssetOracle` entry must exist.
    // Validate against the base config, not the per-spoke override.
    let Some(oracle_config) = storage::get_asset_oracle(&env, quote) else {
        panic_with_error!(&env, OracleError::InvalidOracleBase)
    };
    match &oracle_config.primary {
        // RedStone-shaped feeds (RedStone, Xoxno) are USD-denominated by construction.
        OracleSourceConfig::RedStone(_) | OracleSourceConfig::Xoxno(_) => {}
        // A Reflector quote source must itself be USD-based (no chaining).
        // Use the base cached at config time; do not call live `base()`.
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

/// TWAP read via Reflector price history, averaged over the returned samples.
/// Reverts when history is missing, insufficient, stale, or contains a
/// non-positive price; there is no spot fallback.
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

    let mut sum: i128 = 0;
    let mut oldest_ts = u64::MAX;
    for pd in history.iter() {
        check_not_future_at(env, cache.ledger_timestamp_secs(), pd.timestamp);
        if pd.price <= 0 {
            panic_with_error!(env, OracleError::InvalidPrice);
        }
        sum = sum
            .checked_add(pd.price)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
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

    // Average over returned samples, not the requested count.
    let raw_price = sum / history.len() as i128;
    OracleObservation {
        price_wad: normalize_positive_price(env, raw_price, config.decimals),
        observed_at: oldest_ts,
        published_at: None,
    }
}
