use super::*;
use crate::constants;

/// ±5% band.
fn sample_tolerance() -> OraclePriceFluctuation {
    OraclePriceFluctuation {
        upper_ratio_bps: 10_500,
        lower_ratio_bps: 9_500,
    }
}

#[test]
fn within_band_returns_midpoint() {
    let env = Env::default();
    let anchor = 100 * constants::WAD;
    let primary = 101 * constants::WAD;
    let price = calculate_final_price(&env, anchor, primary, &sample_tolerance());
    assert_eq!(price, (anchor + primary) / 2);
}

#[test]
fn equal_feeds_return_that_price() {
    let env = Env::default();
    let p = 100 * constants::WAD;
    assert_eq!(calculate_final_price(&env, p, p, &sample_tolerance()), p);
}

#[test]
#[should_panic(expected = "Error(Contract, #205)")]
fn beyond_band_panics() {
    let env = Env::default();
    let tight = OraclePriceFluctuation {
        upper_ratio_bps: 10_020,
        lower_ratio_bps: 9_980,
    };
    let _ = calculate_final_price(&env, 100 * constants::WAD, 200 * constants::WAD, &tight);
}

#[test]
#[should_panic(expected = "Error(Contract, #205)")]
fn zero_anchor_is_out_of_band_panics() {
    let env = Env::default();
    let _ = calculate_final_price(&env, 0, 100 * constants::WAD, &sample_tolerance());
}

#[test]
#[should_panic(expected = "Error(Contract, #205)")]
fn degenerate_anchor_overflow_is_out_of_band_panics() {
    // A near-zero anchor against a near-maximum primary overflows the fixed-point
    // narrowing; the ratio short-circuits to out-of-band, so the read reverts
    // rather than panicking with MathOverflow.
    let env = Env::default();
    let _ = calculate_final_price(
        &env,
        1,
        constants::MAX_REASONABLE_PRICE_WAD,
        &sample_tolerance(),
    );
}
