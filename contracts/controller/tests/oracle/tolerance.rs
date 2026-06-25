use super::*;
use crate::cache::Cache;
use crate::constants;
use crate::oracle::policy::OraclePolicy;

fn sample_tolerance() -> OraclePriceFluctuation {
    OraclePriceFluctuation {
        first_upper_ratio_bps: 10_200,
        first_lower_ratio_bps: 9_800,
        last_upper_ratio_bps: 10_500,
        last_lower_ratio_bps: 9_500,
    }
}

#[test]
fn test_is_within_anchor_zero_anchor_returns_false() {
    let env = Env::default();
    assert!(!is_within_anchor(&env, 0, 100 * constants::WAD, 200, 200));
}

#[test]
fn test_is_within_anchor_degenerate_anchor_is_out_of_band_no_panic() {
    let env = Env::default();
    // A valid but near-zero anchor (1) against a near-maximum primary makes
    // primary * RAY overflow the i128 narrowing in mul_div_half_up. The
    // short-circuit must report out-of-band (false) rather than panicking
    // with MathOverflow, so divergence-tolerant policies degrade gracefully.
    assert!(!is_within_anchor(
        &env,
        1,
        constants::MAX_REASONABLE_PRICE_WAD,
        10_500,
        9_500,
    ));
}

#[test]
fn test_calculate_final_price_anchor_only() {
    let env = Env::default();
    let cache = Cache::build(&env, OraclePolicy::View);
    let price = calculate_final_price(&cache, Some(500), None, &sample_tolerance());
    assert_eq!(price, 500);
}

#[test]
fn test_calculate_final_price_first_band_risk_increasing_uses_midpoint() {
    let env = Env::default();
    let cache = Cache::build(&env, OraclePolicy::RiskIncreasing);
    let anchor = 100 * constants::WAD;
    let primary = 101 * constants::WAD;
    let price = calculate_final_price(&cache, Some(anchor), Some(primary), &sample_tolerance());
    assert_eq!(price, (anchor + primary) / 2);
}

#[test]
fn test_calculate_final_price_first_band_risk_decreasing_keeps_primary() {
    let env = Env::default();
    let cache = Cache::build(&env, OraclePolicy::RiskDecreasing);
    let primary = 101 * constants::WAD;
    let price = calculate_final_price(
        &cache,
        Some(100 * constants::WAD),
        Some(primary),
        &sample_tolerance(),
    );
    assert_eq!(price, primary);
}

#[test]
#[should_panic]
fn test_calculate_final_price_none_none_panics() {
    let env = Env::default();
    let cache = Cache::build(&env, OraclePolicy::View);
    let _ = calculate_final_price(&cache, None, None, &sample_tolerance());
}

#[test]
#[should_panic]
fn test_calculate_final_price_unsafe_deviation_liquidation_panics() {
    let env = Env::default();
    let cache = Cache::build(&env, OraclePolicy::Liquidation);
    let tight = OraclePriceFluctuation {
        first_upper_ratio_bps: 10_010,
        first_lower_ratio_bps: 9_990,
        last_upper_ratio_bps: 10_020,
        last_lower_ratio_bps: 9_980,
    };
    let _ = calculate_final_price(
        &cache,
        Some(100 * constants::WAD),
        Some(200 * constants::WAD),
        &tight,
    );
}

#[test]
fn test_calculate_final_price_unsafe_deviation_risk_decreasing_keeps_primary() {
    let env = Env::default();
    let cache = Cache::build(&env, OraclePolicy::RiskDecreasing);
    let tight = OraclePriceFluctuation {
        first_upper_ratio_bps: 10_010,
        first_lower_ratio_bps: 9_990,
        last_upper_ratio_bps: 10_020,
        last_lower_ratio_bps: 9_980,
    };
    let primary = 200 * constants::WAD;
    let price = calculate_final_price(&cache, Some(100 * constants::WAD), Some(primary), &tight);
    assert_eq!(price, primary);
}
