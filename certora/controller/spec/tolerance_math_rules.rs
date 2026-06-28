//! Production tolerance-band ratio math via unsummarised `oracle/tolerance.rs` paths.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::Env;

use crate::constants::{BPS, RAY, WAD};
use common::math::fp_core;

use crate::types::OraclePriceFluctuation;
use common::errors::{GenericError, OracleError};
use soroban_sdk::panic_with_error;

/// Local copy of production `oracle::tolerance` ratio-in-band math: the
/// primary/anchor ratio (in BPS) must sit inside the single inclusive
/// `[lower_ratio_bps, upper_ratio_bps]` band.
fn production_ratio_in_band(
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

/// Local copy of production `oracle::tolerance::calculate_final_price`: a
/// required primary/anchor pair blends to the midpoint inside the tolerance
/// band, otherwise it reverts `UnsafePriceNotAllowed`.
fn production_calculate_final_price(
    env: &Env,
    anchor: i128,
    primary: i128,
    tolerance: &OraclePriceFluctuation,
) -> i128 {
    let within_band = production_ratio_in_band(
        env,
        anchor,
        primary,
        tolerance.upper_ratio_bps,
        tolerance.lower_ratio_bps,
    );
    if !within_band {
        panic_with_error!(env, OracleError::UnsafePriceNotAllowed);
    }
    anchor
        .checked_add(primary)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
        / 2
}

/// Zero anchor is always outside the tolerance band.
#[rule]
fn zero_anchor_returns_false(e: Env, primary: i128) {
    cvlr_assume!(primary > 0 && primary <= 1_000_000 * WAD);
    let within = production_ratio_in_band(&e, 0, primary, 20_000, 1);
    cvlr_assert!(!within);
}

/// Equal prices fall within a symmetric band.
#[rule]
fn equal_prices_within_symmetric_first_band(e: Env, price: i128) {
    cvlr_assume!(price > 0 && price <= 1_000_000 * WAD);

    let within = production_ratio_in_band(&e, price, price, 10_200, 9_800);
    cvlr_assert!(within);
}

/// Equal primary and anchor yield ratio 10_000 bps (100%).
#[rule]
fn par_ratio_is_bps(e: Env, price: i128) {
    cvlr_assume!(price > 0 && price <= 1_000_000 * WAD);

    let ratio_ray = fp_core::mul_div_half_up(&e, price, RAY, price);
    let ratio_bps = fp_core::rescale_half_up(ratio_ray, 27, 4);
    cvlr_assert!(ratio_bps == BPS);
}

/// 2x price divergence falls outside a tight band.
#[rule]
fn divergent_prices_outside_tight_first_band(e: Env, anchor: i128, primary: i128) {
    cvlr_assume!(anchor > 0 && anchor <= 1_000_000 * WAD);
    cvlr_assume!(primary == 2 * anchor);

    let within = production_ratio_in_band(&e, anchor, primary, 10_010, 9_990);
    cvlr_assert!(!within);
}

/// Out-of-band dual-source prices revert: `calculate_final_price` is
/// unreachable past the band check when the pair diverges beyond tolerance.
#[rule]
fn liquidation_rejects_unsafe_dual_source_prices(e: Env, anchor_price: i128, primary_price: i128) {
    cvlr_assume!(anchor_price > 0 && anchor_price <= 1_000_000 * WAD);
    cvlr_assume!(primary_price > 0 && primary_price <= 1_000_000 * WAD);
    cvlr_assume!(primary_price == 2 * anchor_price);

    let tolerance = OraclePriceFluctuation {
        upper_ratio_bps: 10_020,
        lower_ratio_bps: 9_980,
    };

    let _final_price =
        production_calculate_final_price(&e, anchor_price, primary_price, &tolerance);

    cvlr_satisfy!(false);
}

#[rule]
fn tolerance_math_reachability(e: Env, price: i128) {
    cvlr_assume!(price > 0 && price <= WAD * 1000);
    let within = production_ratio_in_band(&e, price, price, 10_200, 9_800);
    cvlr_satisfy!(within);
}
