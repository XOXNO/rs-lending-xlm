//! TWAP read via Reflector's `prices` entry point. Aggregates the
//! window into an integer-mean price (rounded toward zero), gates the
//! result on staleness + minimum-observations, and falls back to a
//! spot read when policy allows.

use common::errors::{GenericError, OracleError};
use common::types::{OracleProviderKind, OracleReadMode, ReflectorSourceConfig};
use soroban_sdk::panic_with_error;

use crate::cache::ControllerCache;
use crate::oracle::observation::{
    check_not_future_at, is_stale, normalize_positive_price, OracleObservation,
};
use crate::oracle::reflector::reflector_prices_call;

use super::{observation_from_price_data, spot::read_spot, to_reflector_asset};

// Minimum non-missing observations a TWAP read must surface to be trusted.
// `ceil(records / 2)` — a partial Reflector outage returning half the
// requested history is still usable. Stricter would let a single-point
// hiccup DoS every consumer; looser would let a TWAP be skewed by very few
// samples.
pub(crate) fn min_twap_observations(records: u32) -> u32 {
    core::cmp::max(1, records.div_ceil(2))
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
        check_not_future_at(env, cache.current_timestamp_ms / 1000, pd.timestamp);
        if pd.price <= 0 {
            has_invalid_price = true;
            continue;
        }
        let candidate =
            observation_from_price_data(env, &pd, config.decimals, OracleReadMode::Spot);
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

    if is_stale(cache.current_timestamp_ms / 1000, oldest_ts, max_stale) {
        return twap_fallback_or_panic(
            cache,
            config,
            required,
            newest_valid,
            OracleError::PriceFeedStale,
        );
    }

    // Euclidean integer division rounds toward zero. With the per-record
    // `i128` headroom (price ≤ 10^36, records ≤ 12) this rounds the TWAP
    // *down*. That's protocol-conservative for collateral valuation
    // (smaller seize) but slightly aggressive for debt valuation (smaller
    // debt → easier-looking HF). The chosen direction is collateral-side
    // because that is the dominant use; debt-side callers that need
    // upward-rounding should compute it explicitly at the call site.
    let raw_price = sum / history.len() as i128;
    Some(OracleObservation {
        price_wad: normalize_positive_price(env, raw_price, config.decimals),
        raw_price,
        raw_decimals: config.decimals,
        observed_at: oldest_ts,
        published_at: None,
        provider: OracleProviderKind::ReflectorSep40,
        read_mode: OracleReadMode::Twap(records),
    })
}

// When TWAP fails, the policy chooses between a graceful fallback
// (spot or newest-valid observation in the window) and a hard panic.
// Liquidation-time reads typically forbid the fallback to deny brief
// outages from masking under-water positions; routine reads allow it.
fn twap_fallback_or_panic(
    cache: &ControllerCache,
    config: &ReflectorSourceConfig,
    required: bool,
    fallback: Option<OracleObservation>,
    err: OracleError,
) -> Option<OracleObservation> {
    if cache.oracle_policy.allows_missing_twap_fallback() {
        fallback.or_else(|| read_spot(cache.env(), config, required))
    } else {
        panic_with_error!(cache.env(), err);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `min_twap_observations` clamps small `records` to 1 (so a 1-sample
    // window doesn't require zero samples), and rounds up otherwise.
    #[test]
    fn test_min_twap_observations_clamps_and_rounds_up() {
        assert_eq!(min_twap_observations(0), 1);
        assert_eq!(min_twap_observations(1), 1);
        assert_eq!(min_twap_observations(2), 1);
        assert_eq!(min_twap_observations(3), 2);
        assert_eq!(min_twap_observations(12), 6);
    }
}
