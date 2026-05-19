//! Certora harness substitute for `controller::oracle::tolerance`.
//!
//! Production `calculate_final_price` calls `is_within_anchor` to
//! decide which branch (safe / aggregator / average) applies. The
//! prover treats the I256-based ratio math as opaque, so
//! [`is_within_anchor`] is replaced with a nondet bool here. The whole
//! file is substituted (not just `is_within_anchor`) so the
//! `calculate_final_price` body in this harness picks up the local
//! summary at name-resolution time.

use common::errors::{GenericError, OracleError};
use common::types::OraclePriceFluctuation;
use cvlr::nondet::nondet;
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
                if cache.oracle_policy.prefers_aggregator_on_deviation() {
                    agg_price
                } else {
                    safe_price
                }
            }
        }
        (Some(agg_price), None) => agg_price,
        (None, Some(safe_price)) => safe_price,
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
