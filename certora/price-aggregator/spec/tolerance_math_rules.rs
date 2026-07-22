//! Production tolerance-band checks through `crate::tolerance`.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::Env;

use common::constants::{BPS, RAY, WAD};
use common::math::fp_core;

use common::types::OracleTolerance;

#[rule]
fn zero_anchor_reverts(e: Env, anchor: i128, primary: i128) {
    cvlr_assume!(anchor == 0);
    cvlr_assume!(primary > 0 && primary <= 1_000_000 * WAD);
    let tolerance = OracleTolerance {
        upper_ratio_bps: 20_000,
        lower_ratio_bps: 1,
    };
    let _ = crate::tolerance::midpoint_if_in_band(&e, anchor, primary, &tolerance);
    cvlr_assert!(false);
}

#[rule]
fn equal_prices_within_symmetric_band(e: Env, price: i128) {
    cvlr_assume!(price > 0 && price <= 1_000_000 * WAD);

    let tolerance = OracleTolerance {
        upper_ratio_bps: 10_200,
        lower_ratio_bps: 9_800,
    };
    let final_price = crate::tolerance::midpoint_if_in_band(&e, price, price, &tolerance);
    cvlr_assert!(final_price == price);
}

#[rule]
fn par_ratio_is_bps(e: Env, price: i128) {
    cvlr_assume!(price > 0 && price <= 1_000_000 * WAD);

    let ratio_ray = fp_core::mul_div_half_up(&e, price, RAY, price);
    let ratio_bps = fp_core::rescale_half_up(ratio_ray, 27, 4);
    cvlr_assert!(ratio_bps == BPS);
}

#[rule]
fn divergent_prices_revert(e: Env, anchor: i128, primary: i128) {
    cvlr_assume!(anchor > 0 && anchor <= 1_000_000 * WAD);
    cvlr_assume!(primary == 2 * anchor);

    let tolerance = OracleTolerance {
        upper_ratio_bps: 10_010,
        lower_ratio_bps: 9_990,
    };
    let _ = crate::tolerance::midpoint_if_in_band(&e, anchor, primary, &tolerance);
    cvlr_assert!(false);
}
