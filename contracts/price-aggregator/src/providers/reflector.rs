//! Reflector SEP-40 price provider: spot or TWAP read, repricing a quoted base
//! into USD. The hard read path reverts on missing/short TWAP history; the
//! soft (status) path maps every per-asset read problem to `None` so
//! diagnostic views never revert. Staleness is owned by the callers (`compose`
//! reverts, `status` flags), not by this reader.

use common::errors::OracleError;
use common::math::fp::Wad;
use common::oracle::observation::{check_not_future_at, normalize_positive_price};
use common::oracle::providers::reflector::{
    min_twap_observations, reflector_lastprice_call, reflector_prices_call, to_reflector_asset,
    twap_mean_price,
};
use common::types::{OracleReadMode, PriceFeedRaw, ReflectorBase, ReflectorSourceConfig};
use common::validation::validate_twap_records;
use soroban_sdk::{panic_with_error, Address};

use crate::config::{is_usd_rooted, require_usd_rooted};
use crate::context::ResolutionContext;
use crate::observation::OracleObservation;
use crate::price;
use crate::status;

/// `soft = false` (hard path): missing/short TWAP history and quoted-base
/// failures revert with their precise error. `soft = true` (status path):
/// they yield `None` and the caller reports an unusable status.
pub(crate) fn read_reflector_source(
    cache: &mut ResolutionContext,
    config: &ReflectorSourceConfig,
    soft: bool,
) -> Option<OracleObservation> {
    let observation = match config.read_mode {
        OracleReadMode::Spot => read_spot(cache, config),
        OracleReadMode::Twap(records) => match read_twap(cache, config, records) {
            Ok(obs) => Some(obs),
            Err(_) if soft => None,
            Err(err) => panic_with_error!(cache.env(), err),
        },
    };
    observation.and_then(|obs| reprice_to_usd(cache, &config.base, obs, soft))
}

/// `None` only in soft mode (unresolvable quote leg); hard mode reverts.
fn reprice_to_usd(
    cache: &mut ResolutionContext,
    base: &ReflectorBase,
    obs: OracleObservation,
    soft: bool,
) -> Option<OracleObservation> {
    match base {
        ReflectorBase::Usd => Some(obs),
        ReflectorBase::Quoted(quote) => {
            let env = cache.env().clone();
            let quote_feed = if soft {
                try_resolve_usd_quote_soft(cache, quote)?
            } else {
                resolve_usd_quote(cache, quote)
            };
            let price_usd = Wad::from(obs.price_wad)
                .mul(&env, Wad::from(quote_feed.price_wad))
                .raw();
            Some(OracleObservation {
                price_wad: price_usd,
                // Freshness is the staler of token and quote legs.
                observed_at: obs.observed_at.min(quote_feed.timestamp),
                published_at: obs.published_at,
            })
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

/// Soft quote resolution for the status path: any failure (missing config,
/// non-USD root, quote cycle, or an invalid quote status) yields `None`
/// instead of reverting the diagnostic view. The quote leg must be fully
/// VALID — fresh, in band, inside its sanity bounds — to back a reprice.
fn try_resolve_usd_quote_soft(
    cache: &mut ResolutionContext,
    quote: &Address,
) -> Option<PriceFeedRaw> {
    if cache.is_resolving(quote) {
        return None;
    }
    let quote_oracle = cache.cached_asset_oracle_opt(quote)?;
    if !is_usd_rooted(&quote_oracle) {
        return None;
    }
    cache.push_resolution(quote);
    let quote_status = status::resolve_price_status(cache, quote);
    cache.pop_resolution();
    if !quote_status.valid {
        return None;
    }
    Some(PriceFeedRaw {
        price_wad: quote_status.final_wad,
        asset_decimals: quote_oracle.asset_decimals,
        timestamp: quote_status.price_timestamp,
    })
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

/// TWAP over returned samples; missing/short history is an `Err` for the
/// caller to revert (hard) or soften (status). Config-invariant violations
/// (record bounds) and future timestamps stay hard in both modes.
fn read_twap(
    cache: &ResolutionContext,
    config: &ReflectorSourceConfig,
    records: u32,
) -> Result<OracleObservation, OracleError> {
    let env = cache.env();
    let now_secs = cache.ledger_timestamp_secs();
    validate_twap_records(env, records);

    let asset = to_reflector_asset(env, &config.asset);
    let Some(history) = reflector_prices_call(env, &config.contract, &asset, records) else {
        return Err(OracleError::ReflectorHistoryEmpty);
    };
    if history.is_empty() {
        return Err(OracleError::ReflectorHistoryEmpty);
    }
    if history.len() < min_twap_observations(records) {
        return Err(OracleError::TwapInsufficientObservations);
    }

    let mut oldest_ts = u64::MAX;
    for price_data in history.iter() {
        check_not_future_at(env, now_secs, price_data.timestamp);
        if price_data.timestamp < oldest_ts {
            oldest_ts = price_data.timestamp;
        }
    }

    // Mean over returned samples (not requested count); shared with governance
    // probe. Staleness of `oldest_ts` is judged by the caller.
    let raw_price = twap_mean_price(env, &history);
    Ok(OracleObservation {
        price_wad: normalize_positive_price(env, raw_price, config.decimals),
        observed_at: oldest_ts,
        published_at: None,
    })
}
