use super::*;
use common::constants;
use soroban_sdk::Env;

fn sample_tolerance() -> OracleTolerance {
    OracleTolerance {
        upper_ratio_bps: 10_500,
        lower_ratio_bps: 9_524,
    }
}

#[test]
fn within_band_accepts_close_prices() {
    let env = Env::default();
    let anchor = 100 * constants::WAD;
    let primary = 101 * constants::WAD;
    assert!(within_tolerance_band(
        &env,
        anchor,
        primary,
        &sample_tolerance()
    ));
    assert_eq!(
        midpoint_price_or_zero(anchor, primary),
        (anchor + primary) / 2
    );
}

#[test]
fn equal_feeds_return_that_price() {
    let p = 100 * constants::WAD;
    assert_eq!(midpoint_price_or_zero(p, p), p);
}

#[test]
fn beyond_band_is_rejected() {
    let env = Env::default();
    let tight = OracleTolerance {
        upper_ratio_bps: 10_020,
        lower_ratio_bps: 9_980,
    };
    assert!(!within_tolerance_band(
        &env,
        100 * constants::WAD,
        200 * constants::WAD,
        &tight
    ));
}

#[test]
fn zero_anchor_is_out_of_band() {
    let env = Env::default();
    assert!(!within_tolerance_band(
        &env,
        0,
        100 * constants::WAD,
        &sample_tolerance()
    ));
}

#[test]
fn degenerate_anchor_overflow_is_out_of_band() {
    let env = Env::default();
    assert!(!within_tolerance_band(
        &env,
        1,
        constants::MAX_REASONABLE_PRICE_WAD,
        &sample_tolerance()
    ));
}

#[test]
fn midpoint_overflow_returns_zero() {
    assert_eq!(midpoint_price_or_zero(i128::MAX, 1), 0);
}

#[test]
fn tolerance_boundary_is_source_order_invariant() {
    let env = Env::default();
    let tolerance = OracleTolerance {
        upper_ratio_bps: 10_500,
        lower_ratio_bps: 9_524,
    };
    let high = 1_070i128;
    let low = 1_019i128;

    assert!(within_tolerance_band(&env, low, high, &tolerance));
    assert_eq!(
        within_tolerance_band(&env, low, high, &tolerance),
        within_tolerance_band(&env, high, low, &tolerance)
    );
}

#[test]
fn tolerance_uses_one_step_bps_rounding() {
    let env = Env::default();
    let tolerance = OracleTolerance {
        upper_ratio_bps: 10_150,
        lower_ratio_bps: 9_852,
    };
    let high = 101_505_000_000_000_000_009_847i128;
    let low = 100_000_000_000_000_000_009_701i128;

    assert!(within_tolerance_band(&env, low, high, &tolerance));
    assert!(within_tolerance_band(&env, high, low, &tolerance));
}
