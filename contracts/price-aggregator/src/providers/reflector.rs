//! Reflector SEP-40 price provider: spot or TWAP read. The hard read path
//! reverts on missing/short TWAP history; the
//! soft path maps every per-asset read problem to `None` for the diagnostic
//! views and for `compose`'s traversal. Three failures revert under either
//! discipline: a record count `validate_twap_records` rejects, an asset ref
//! `to_reflector_asset` cannot express, and a Reflector contract that reverts
//! at read time. Staleness is owned by the engine, not by this reader.
//!
//! Repricing a non-USD base used to live here. It is `PriceSource::Scaled`'s
//! job now, which is why this reader always reads a feed in whatever unit it
//! natively publishes.

use common::errors::OracleError;
use common::oracle::observation::is_future_at;
use common::oracle::providers::reflector::{
    min_twap_observations, reflector_lastprice_call, reflector_prices_call, to_reflector_asset,
    try_twap_mean_price,
};
use common::types::{OracleReadMode, ReflectorSourceConfig};
use common::validation::validate_twap_records;
use soroban_sdk::panic_with_error;

use crate::context::ResolutionContext;
use crate::observation::OracleObservation;

/// `soft = false` (hard path): missing/short TWAP history and quoted-base
/// failures revert with their precise error. `soft = true` (status path):
/// they yield `None` and the caller reports an unusable status.
pub(crate) fn read_reflector_source(
    cache: &mut ResolutionContext,
    config: &ReflectorSourceConfig,
    soft: bool,
) -> Option<OracleObservation> {
    let observation = match config.read_mode {
        OracleReadMode::Spot => read_spot(cache, config, soft),
        OracleReadMode::Twap(records) => match read_twap(cache, config, records) {
            Ok(obs) => Some(obs),
            Err(_) if soft => None,
            Err(err) => panic_with_error!(cache.env(), err),
        },
    };
    observation
}

/// Spot read via Reflector `lastprice`. `None` when the feed carries no price,
/// or — in soft mode only — when the payload is rejected.
fn read_spot(
    cache: &ResolutionContext,
    config: &ReflectorSourceConfig,
    soft: bool,
) -> Option<OracleObservation> {
    let env = cache.env();
    let now_secs = cache.ledger_timestamp_secs();
    let asset = to_reflector_asset(env, &config.asset);
    let price_data = reflector_lastprice_call(env, &config.contract, &asset)?;
    OracleObservation::from_reflector(env, now_secs, &price_data, config.decimals, soft)
}

/// TWAP over returned samples. Every present-but-invalid condition (missing or
/// short history, a future timestamp, a non-positive/overflowing sample) is an
/// `Err` for the caller to revert (hard path) or soften (status path). The
/// record-count check and the asset-ref conversion run ahead of all of that and
/// panic under either discipline, as does a `prices` call the Reflector
/// contract itself reverts.
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
        if is_future_at(now_secs, price_data.timestamp) {
            return Err(OracleError::PriceFeedStale);
        }
        if price_data.timestamp < oldest_ts {
            oldest_ts = price_data.timestamp;
        }
    }

    // Mean over returned samples (not requested count); shared with governance
    // probe. Staleness of `oldest_ts` is judged by the caller.
    let raw_price = try_twap_mean_price(&history).ok_or(OracleError::InvalidPrice)?;
    let price_wad =
        common::oracle::observation::try_normalize_positive_price(raw_price, config.decimals)
            .ok_or(OracleError::InvalidPrice)?;
    Ok(OracleObservation {
        price_wad,
        observed_at: oldest_ts,
        published_at: None,
    })
}
