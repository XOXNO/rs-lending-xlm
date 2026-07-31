use super::*;
use crate::constants::WAD_DECIMALS;
use soroban_sdk::{Env, U256};

#[test]
fn is_stale_false_at_exact_max_age() {
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
    validate_timestamp(&env, 1_000, 800, 60);
}

#[test]
#[should_panic]
fn validate_timestamp_rejects_future_skew() {
    let env = Env::default();
    validate_timestamp(&env, 1_000, 1_100, 60);
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

#[test]
#[should_panic(expected = "#33")]
fn test_u256_to_i128_rejects_above_u128() {
    let env = Env::default();

    let big = U256::from_u128(&env, u128::MAX).add(&U256::from_u32(&env, 2));
    let _ = u256_to_i128(&env, &big);
}

#[test]
fn try_normalize_positive_price_softens_invalid() {
    assert_eq!(try_normalize_positive_price(0, 7), None);
    assert_eq!(try_normalize_positive_price(-1, 7), None);

    let env = Env::default();
    assert_eq!(
        try_normalize_positive_price(1_000, 7),
        Some(normalize_positive_price(&env, 1_000, 7))
    );

    assert_eq!(try_normalize_positive_price(i128::MAX, 7), None);

    assert_eq!(
        try_normalize_positive_price(1_000, WAD_DECIMALS),
        Some(1_000)
    );

    assert_eq!(try_normalize_positive_price(1_000, WAD_DECIMALS + 1), None);
}

#[test]
fn is_future_at_matches_skew_window() {
    assert!(!is_future_at(1_000, 1_000 + MAX_FUTURE_SKEW_SECONDS));
    assert!(is_future_at(1_000, 1_000 + MAX_FUTURE_SKEW_SECONDS + 1));

    assert!(!is_future_at(u64::MAX, u64::MAX));
}

#[test]
fn try_u256_to_i128_softens_overflow() {
    let env = Env::default();
    assert_eq!(try_u256_to_i128(&U256::from_u32(&env, 42)), Some(42));

    assert_eq!(
        try_u256_to_i128(&U256::from_u128(&env, i128::MAX as u128)),
        Some(i128::MAX)
    );
    let too_big = U256::from_u128(&env, i128::MAX as u128).add(&U256::from_u32(&env, 1));
    assert_eq!(try_u256_to_i128(&too_big), None);
}
