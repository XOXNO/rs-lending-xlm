// TWAP read via Reflector prices.

use common::constants::MS_PER_SECOND;
use common::errors::{GenericError, OracleError};
use common::events::{emit_oracle_twap_degraded, OracleTwapDegradedEvent};
use common::types::ReflectorSourceConfig;
use soroban_sdk::{assert_with_error, panic_with_error};

use super::reflector_prices_call;
use crate::cache::ControllerCache;
use crate::oracle::observation::{
    check_not_future_at, is_stale, normalize_positive_price, OracleObservation, MAX_TWAP_RECORDS,
};

use super::{observation_from_price_data, spot::read_spot, to_reflector_asset};

// Min observations for trusted TWAP. Floor of 2 rules out single-sample
// "TWAPs"; larger windows accept partial history above ceil(records/2).
pub(crate) fn min_twap_observations(records: u32) -> u32 {
    core::cmp::max(2, records.div_ceil(2))
}

pub(crate) fn read_twap(
    cache: &mut ControllerCache,
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
        check_not_future_at(
            env,
            cache.current_timestamp_ms / MS_PER_SECOND,
            pd.timestamp,
        );
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

    if is_stale(
        cache.current_timestamp_ms / MS_PER_SECOND,
        oldest_ts,
        max_stale,
    ) {
        return twap_fallback_or_panic(
            cache,
            config,
            required,
            newest_valid,
            OracleError::PriceFeedStale,
        );
    }

    // Average of actually-returned samples (AverageAvailable semantics).
    let raw_price = sum / history.len() as i128;
    Some(OracleObservation {
        price_wad: normalize_positive_price(env, raw_price, config.decimals),
        observed_at: oldest_ts,
        published_at: None,
    })
}

// TWAP fallback path (used when the primary spot feed is stale or degraded).
// Returns the computed TWAP observation or falls back to the last known price.
fn twap_fallback_or_panic(
    cache: &ControllerCache,
    config: &ReflectorSourceConfig,
    required: bool,
    fallback: Option<OracleObservation>,
    err: OracleError,
) -> Option<OracleObservation> {
    if cache.oracle_policy.allows_missing_twap_fallback() {
        emit_oracle_twap_degraded(
            cache.env(),
            OracleTwapDegradedEvent {
                oracle: config.contract.clone(),
                reason_code: err as u32,
            },
        );
        fallback.or_else(|| read_spot(cache.env(), config, required))
    } else {
        panic_with_error!(cache.env(), err);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_min_twap_observations_clamps_and_rounds_up() {
        assert_eq!(min_twap_observations(0), 2);
        assert_eq!(min_twap_observations(1), 2);
        assert_eq!(min_twap_observations(2), 2);
        assert_eq!(min_twap_observations(3), 2);
        assert_eq!(min_twap_observations(4), 2);
        assert_eq!(min_twap_observations(5), 3);
        assert_eq!(min_twap_observations(12), 6);
    }
}
