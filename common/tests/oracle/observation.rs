use super::*;
use soroban_sdk::{Env, U256};

#[test]
fn is_stale_false_at_exact_max_age() {
    // elapsed == max_stale is fresh (strict `>`).
    assert!(!is_stale(160, 100, 60));
}

#[test]
fn is_stale_true_past_max_age() {
    assert!(is_stale(161, 100, 60));
}

#[test]
fn is_stale_false_when_feed_not_in_past() {
    assert!(!is_stale(100, 100, 60));
    assert!(!is_stale(100, 200, 60));
}

#[test]
fn millis_to_seconds_divides_by_thousand() {
    assert_eq!(millis_to_seconds(1_500), 1);
    assert_eq!(millis_to_seconds(60_000), 60);
}

#[test]
fn normalize_scales_token_to_wad() {
    let env = Env::default();
    // price 1 at 6 decimals -> 1 * 10^(18-6) WAD.
    assert_eq!(normalize_positive_price(&env, 1, 6), 1_000_000_000_000);
}

#[test]
#[should_panic]
fn normalize_rejects_nonpositive() {
    let env = Env::default();
    normalize_positive_price(&env, 0, 6);
}

#[test]
fn u256_to_i128_roundtrips() {
    let env = Env::default();
    let v = U256::from_u128(&env, 12_345);
    assert_eq!(u256_to_i128(&env, &v), 12_345);
}

#[test]
fn validate_timestamp_accepts_fresh() {
    let env = Env::default();
    validate_timestamp(&env, 1_000, 990, 60);
}

#[test]
#[should_panic]
fn validate_timestamp_rejects_stale() {
    let env = Env::default();
    validate_timestamp(&env, 1_000, 800, 60); // elapsed 200 > 60
}

#[test]
#[should_panic]
fn validate_timestamp_rejects_future_skew() {
    let env = Env::default();
    validate_timestamp(&env, 1_000, 1_100, 60); // 100 > MAX_FUTURE_SKEW_SECONDS
}

#[test]
#[should_panic]
fn check_not_future_at_rejects_skew() {
    let env = Env::default();
    check_not_future_at(&env, 1_000, 1_100);
}

#[test]
fn validate_positive_price_timestamps_returns_wad() {
    let env = Env::default();
    let timestamps = [990u64, 995u64];
    let out = validate_positive_price_timestamps(&env, 1, 6, 1_000, &timestamps, 60);
    assert_eq!(out, 1_000_000_000_000);
}
