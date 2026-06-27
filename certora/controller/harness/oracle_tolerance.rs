//! Certora harness for `controller::oracle::tolerance`.
//!
//! `calculate_final_price` and `is_within_anchor` perform fixed-point ratio
//! math that is expensive for the prover. This harness replaces the tolerance
//! decision with a sound nondet bool while preserving the control-flow
//! branches and panic conditions, leaving `oracle/tolerance.rs` untouched.

use crate::types::OraclePriceFluctuation;
use common::errors::{GenericError, OracleError};
use cvlr::nondet::nondet;
use soroban_sdk::{panic_with_error, Env};

use crate::cache::Cache;

pub(crate) struct FinalPrice {
    pub price_wad: i128,
    pub within_first: bool,
    pub within_second: bool,
}

pub(crate) fn calculate_final_price(
    cache: &Cache,
    aggregator: Option<i128>,
    safe: Option<i128>,
    tolerance: &OraclePriceFluctuation,
) -> FinalPrice {
    let env = cache.env();
    match (aggregator, safe) {
        (Some(agg_price), Some(safe_price)) => {
            let within_first = is_within_anchor(
                env,
                agg_price,
                safe_price,
                tolerance.first_upper_ratio_bps,
                tolerance.first_lower_ratio_bps,
            );
            let within_second = is_within_anchor(
                env,
                agg_price,
                safe_price,
                tolerance.last_upper_ratio_bps,
                tolerance.last_lower_ratio_bps,
            );
            let price_wad = if within_first {
                safe_price
            } else if within_second {
                agg_price
                    .checked_add(safe_price)
                    .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
                    / 2
            } else {
                if !cache.oracle_policy.allows_unsafe_deviation() {
                    panic_with_error!(env, OracleError::UnsafePriceNotAllowed);
                }
                safe_price
            };
            FinalPrice {
                price_wad,
                within_first,
                within_second,
            }
        }
        (Some(agg_price), None) => FinalPrice {
            price_wad: agg_price,
            within_first: false,
            within_second: false,
        },
        (None, Some(safe_price)) => FinalPrice {
            price_wad: safe_price,
            within_first: false,
            within_second: false,
        },
        (None, None) => {
            panic_with_error!(env, OracleError::NoLastPrice);
        }
    }
}

// Summary: tests whether `safe / aggregator` (in bps) sits inside
// `[lower_bound_ratio, upper_bound_ratio]`. The real implementation
// does an I256 ratio + BPS rescale that the prover can't traverse;
// returning a free nondet bool is sound because the boundary branch
// each call selects is determined by the inputs.
pub(crate) fn is_within_anchor(
    _env: &Env,
    aggregator: i128,
    _safe: i128,
    _upper_bound_ratio: u32,
    _lower_bound_ratio: u32,
) -> bool {
    // Preserve the `aggregator == 0` short-circuit from production:
    // the ratio is undefined, so production returns false.
    if aggregator == 0 {
        return false;
    }
    nondet()
}
