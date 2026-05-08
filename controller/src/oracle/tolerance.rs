use common::errors::{GenericError, OracleError};
use common::fp::Ray;
use common::fp_core;
use common::types::OraclePriceFluctuation;
use soroban_sdk::{panic_with_error, Env};

use crate::cache::ControllerCache;

pub(crate) fn calculate_final_price(
    cache: &ControllerCache,
    aggregator: Option<i128>,
    safe: Option<i128>,
    tolerance: &OraclePriceFluctuation,
) -> i128 {
    let env = cache.env();
    match (aggregator, safe) {
        (Some(agg_price), Some(safe_price)) => {
            if is_within_anchor(
                env,
                agg_price,
                safe_price,
                tolerance.first_upper_ratio_bps,
                tolerance.first_lower_ratio_bps,
            ) {
                safe_price
            } else if is_within_anchor(
                env,
                agg_price,
                safe_price,
                tolerance.last_upper_ratio_bps,
                tolerance.last_lower_ratio_bps,
            ) {
                agg_price
                    .checked_add(safe_price)
                    .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
                    / 2
            } else {
                if !cache.oracle_policy.allows_unsafe_deviation() {
                    panic_with_error!(env, OracleError::UnsafePriceNotAllowed);
                }
                safe_price
            }
        }
        (Some(agg_price), None) => agg_price,
        (None, Some(safe_price)) => safe_price,
        (None, None) => {
            panic_with_error!(env, OracleError::NoLastPrice);
        }
    }
}

crate::summarized!(
    is_within_anchor_summary,
    pub(crate) fn is_within_anchor(
        env: &Env,
        aggregator: i128,
        safe: i128,
        upper_bound_ratio: u32,
        lower_bound_ratio: u32,
    ) -> bool {
        if aggregator == 0 {
            return false;
        }
        let ratio_ray = Ray::from_raw(safe)
            .div(env, Ray::from_raw(aggregator))
            .raw();
        let ratio_bps = fp_core::rescale_half_up(ratio_ray, 27, 4);
        let upper = i128::from(upper_bound_ratio);
        let lower = i128::from(lower_bound_ratio);

        ratio_bps <= upper && ratio_bps >= lower
    }
);
