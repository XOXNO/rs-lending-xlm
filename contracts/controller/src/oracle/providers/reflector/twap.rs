// TWAP read via Reflector prices.

use common::errors::{GenericError, OracleError};
use common::oracle::observation::{
    check_not_future_at, is_stale, normalize_positive_price, MAX_TWAP_RECORDS,
};
use common::oracle::providers::reflector::{
    min_twap_observations, reflector_prices_call, to_reflector_asset,
};
use controller_interface::types::ReflectorSourceConfig;
use soroban_sdk::{assert_with_error, panic_with_error};

use crate::cache::Cache;
use crate::events::OracleTwapDegradedEvent;
use crate::oracle::observation::OracleObservation;

use super::{observation_from_price_data, spot::read_spot};

pub(crate) fn read_twap(
    cache: &mut Cache,
    config: &ReflectorSourceConfig,
    records: u32,
    max_stale: u64,
    required: bool,
) -> Option<OracleObservation> {
    if records == 0 {
        return twap_fallback_or_panic(
            cache,
            config,
            required,
            None,
            OracleError::TwapInsufficientObservations,
        );
    }
    assert_with_error!(
        cache.env(),
        records <= MAX_TWAP_RECORDS,
        OracleError::InvalidOracleTokenType
    );

    let env = cache.env();
    let asset = to_reflector_asset(env, &config.asset);
    let Some(history) = reflector_prices_call(env, &config.contract, &asset, records) else {
        return twap_fallback_or_panic(
            cache,
            config,
            required,
            None,
            OracleError::ReflectorHistoryEmpty,
        );
    };
    if history.is_empty() {
        return twap_fallback_or_panic(
            cache,
            config,
            required,
            None,
            OracleError::ReflectorHistoryEmpty,
        );
    }

    let mut sum: i128 = 0;
    let mut oldest_ts = u64::MAX;
    let mut newest_valid: Option<OracleObservation> = None;
    let mut has_invalid_price = false;
    for pd in history.iter() {
        check_not_future_at(env, cache.ledger_timestamp_secs(), pd.timestamp);
        if pd.price <= 0 {
            has_invalid_price = true;
            continue;
        }
        let candidate = observation_from_price_data(env, &pd, config.decimals);
        if newest_valid
            .as_ref()
            .is_none_or(|current| candidate.observed_at > current.observed_at)
        {
            newest_valid = Some(candidate);
        }
        sum = sum
            .checked_add(pd.price)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
        if pd.timestamp < oldest_ts {
            oldest_ts = pd.timestamp;
        }
    }

    if has_invalid_price {
        return twap_fallback_or_panic(
            cache,
            config,
            required,
            newest_valid,
            OracleError::InvalidPrice,
        );
    }

    if history.len() < min_twap_observations(records) {
        return twap_fallback_or_panic(
            cache,
            config,
            required,
            newest_valid,
            OracleError::TwapInsufficientObservations,
        );
    }

    if is_stale(cache.ledger_timestamp_secs(), oldest_ts, max_stale) {
        return twap_fallback_or_panic(
            cache,
            config,
            required,
            newest_valid,
            OracleError::PriceFeedStale,
        );
    }

    // Average over returned samples, not the requested count.
    let raw_price = sum / history.len() as i128;
    Some(OracleObservation {
        price_wad: normalize_positive_price(env, raw_price, config.decimals),
        observed_at: oldest_ts,
        published_at: None,
    })
}

// Policy-controlled fallback when TWAP history is unavailable.
fn twap_fallback_or_panic(
    cache: &Cache,
    config: &ReflectorSourceConfig,
    required: bool,
    fallback: Option<OracleObservation>,
    err: OracleError,
) -> Option<OracleObservation> {
    if cache.oracle_policy.allows_degraded_dual_source() {
        OracleTwapDegradedEvent {
            oracle: config.contract.clone(),
            reason_code: err as u32,
        }
        .publish(cache.env());
        fallback.or_else(|| read_spot(cache.env(), config, required))
    } else {
        panic_with_error!(cache.env(), err);
    }
}
