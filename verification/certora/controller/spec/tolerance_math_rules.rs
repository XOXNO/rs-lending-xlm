/// Production tolerance-band ratio math (unsummarised).
///
/// These rules exercise the real `primary/anchor` ratio computation from
/// `oracle/tolerance.rs` via `fp_core`, not the Certora harness nondet
/// replacement used in `oracle.conf` policy-branch rules.
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::Env;

use crate::constants::{BPS, RAY, WAD};
use common::math::fp_core;

use crate::types::OraclePriceFluctuation;
use common::errors::{GenericError, OracleError};
use soroban_sdk::panic_with_error;

use crate::cache::Cache;

/// Mirrors production `is_within_anchor` (`oracle/tolerance.rs`).
fn production_is_within_anchor(
    env: &Env,
    anchor: i128,
    primary: i128,
    upper_bound_ratio: u32,
    lower_bound_ratio: u32,
) -> bool {
    if anchor == 0 {
        return false;
    }
    let ratio_ray = fp_core::mul_div_half_up(env, primary, RAY, anchor);
    let ratio_bps = fp_core::rescale_half_up(ratio_ray, 27, 4);
    let upper = i128::from(upper_bound_ratio);
    let lower = i128::from(lower_bound_ratio);
    ratio_bps <= upper && ratio_bps >= lower
}

/// Mirrors production `calculate_final_price` with real band math.
fn production_calculate_final_price(
    cache: &Cache,
    anchor: Option<i128>,
    primary: Option<i128>,
    tolerance: &OraclePriceFluctuation,
) -> i128 {
    let env = cache.env();
    match (anchor, primary) {
        (Some(anchor_price), Some(primary_price)) => {
            if production_is_within_anchor(
                env,
                anchor_price,
                primary_price,
                tolerance.first_upper_ratio_bps,
                tolerance.first_lower_ratio_bps,
            ) {
                primary_price
            } else if production_is_within_anchor(
                env,
                anchor_price,
                primary_price,
                tolerance.last_upper_ratio_bps,
                tolerance.last_lower_ratio_bps,
            ) {
                anchor_price
                    .checked_add(primary_price)
                    .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
                    / 2
            } else if !cache.oracle_policy.allows_unsafe_deviation() {
                panic_with_error!(env, OracleError::UnsafePriceNotAllowed);
            } else {
                primary_price
            }
        }
        (Some(anchor_price), None) => anchor_price,
        (None, Some(primary_price)) => primary_price,
        (None, None) => panic_with_error!(env, OracleError::NoLastPrice),
    }
}

#[rule]
fn zero_anchor_returns_false(e: Env, primary: i128) {
    cvlr_assume!(primary > 0 && primary <= 1_000_000 * WAD);
    let within = production_is_within_anchor(&e, 0, primary, 20_000, 1);
    cvlr_assert!(!within);
}

#[rule]
fn equal_prices_within_symmetric_first_band(e: Env, price: i128) {
    cvlr_assume!(price > 0 && price <= 1_000_000 * WAD);

    let within = production_is_within_anchor(&e, price, price, 10_200, 9_800);
    cvlr_assert!(within);
}

#[rule]
fn par_ratio_is_bps(e: Env, price: i128) {
    cvlr_assume!(price > 0 && price <= 1_000_000 * WAD);

    let ratio_ray = fp_core::mul_div_half_up(&e, price, RAY, price);
    let ratio_bps = fp_core::rescale_half_up(ratio_ray, 27, 4);
    cvlr_assert!(ratio_bps == BPS);
}

#[rule]
fn divergent_prices_outside_tight_first_band(e: Env, anchor: i128, primary: i128) {
    cvlr_assume!(anchor > 0 && anchor <= 1_000_000 * WAD);
    cvlr_assume!(primary == 2 * anchor);

    let within = production_is_within_anchor(&e, anchor, primary, 10_010, 9_990);
    cvlr_assert!(!within);
}

/// Permissive-mode return when both prices diverge beyond the last band.
#[rule]
fn beyond_tolerance_permissive_returns_primary(e: Env, anchor_price: i128, primary_price: i128) {
    cvlr_assume!(anchor_price > 0 && anchor_price <= 1_000_000 * WAD);
    cvlr_assume!(primary_price > 0 && primary_price <= 1_000_000 * WAD);
    cvlr_assume!(primary_price == 2 * anchor_price);

    let cache = crate::cache::Cache::new(&e, crate::oracle::policy::OraclePolicy::RiskDecreasing);
    let tolerance = OraclePriceFluctuation {
        first_upper_ratio_bps: 10_010,
        first_lower_ratio_bps: 9_990,
        last_upper_ratio_bps: 10_020,
        last_lower_ratio_bps: 9_980,
    };

    let final_price = production_calculate_final_price(
        &cache,
        Some(anchor_price),
        Some(primary_price),
        &tolerance,
    );

    cvlr_assert!(final_price == primary_price);
}

/// Strict liquidation policy reverts on dual-source divergence beyond the last band.
#[rule]
fn liquidation_rejects_unsafe_dual_source_prices(e: Env, anchor_price: i128, primary_price: i128) {
    cvlr_assume!(anchor_price > 0 && anchor_price <= 1_000_000 * WAD);
    cvlr_assume!(primary_price > 0 && primary_price <= 1_000_000 * WAD);
    cvlr_assume!(primary_price == 2 * anchor_price);

    let cache = crate::cache::Cache::new(&e, crate::oracle::policy::OraclePolicy::Liquidation);
    let tolerance = OraclePriceFluctuation {
        first_upper_ratio_bps: 10_010,
        first_lower_ratio_bps: 9_990,
        last_upper_ratio_bps: 10_020,
        last_lower_ratio_bps: 9_980,
    };

    let _final_price = production_calculate_final_price(
        &cache,
        Some(anchor_price),
        Some(primary_price),
        &tolerance,
    );

    cvlr_satisfy!(false);
}

#[rule]
fn tolerance_math_reachability(e: Env, price: i128) {
    cvlr_assume!(price > 0 && price <= WAD * 1000);
    let within = production_is_within_anchor(&e, price, price, 10_200, 9_800);
    cvlr_satisfy!(within);
}
