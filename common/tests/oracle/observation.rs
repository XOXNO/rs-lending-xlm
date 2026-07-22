use super::*;
use crate::constants::WAD_DECIMALS;
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

// test_u256_to_i128_rejects_above_u128 (+1) common/src/oracle/observation.rs:97
#[test]
#[should_panic(expected = "#33")]
fn test_u256_to_i128_rejects_above_u128() {
    let env = Env::default();
    // u128::MAX + 2 overflows the u128 domain, so to_u128() returns None.
    let big = U256::from_u128(&env, u128::MAX).add(&U256::from_u32(&env, 2));
    let _ = u256_to_i128(&env, &big);
}

#[test]
fn try_normalize_positive_price_softens_invalid() {
    // Non-positive → None.
    assert_eq!(try_normalize_positive_price(0, 7), None);
    assert_eq!(try_normalize_positive_price(-1, 7), None);
    // Valid upscale matches the panicking form.
    let env = Env::default();
    assert_eq!(
        try_normalize_positive_price(1_000, 7),
        Some(normalize_positive_price(&env, 1_000, 7))
    );
    // i128::MAX upscaled by 10^11 overflows → None.
    assert_eq!(try_normalize_positive_price(i128::MAX, 7), None);
    // Boundary: WAD_DECIMALS (18) is the max valid decimals — a pure identity
    // upscale (10^0 = 1), so it must return Some, not None. Pins the guard's
    // `>` (not `==`/`>=`): at exactly 18 the price passes through.
    assert_eq!(try_normalize_positive_price(1_000, WAD_DECIMALS), Some(1_000));
    // One past the boundary rejects instead of underflowing `WAD_DECIMALS -
    // decimals` (which would panic in the soft path).
    assert_eq!(try_normalize_positive_price(1_000, WAD_DECIMALS + 1), None);
}

#[test]
fn is_future_at_matches_skew_window() {
    // Within skew is not future; beyond skew is.
    assert!(!is_future_at(1_000, 1_000 + MAX_FUTURE_SKEW_SECONDS));
    assert!(is_future_at(1_000, 1_000 + MAX_FUTURE_SKEW_SECONDS + 1));
    // now + skew overflow → nothing is future.
    assert!(!is_future_at(u64::MAX, u64::MAX));
}

#[test]
fn try_u256_to_i128_softens_overflow() {
    let env = Env::default();
    assert_eq!(try_u256_to_i128(&U256::from_u32(&env, 42)), Some(42));
    // Boundary: exactly i128::MAX is accepted (pins the guard's `<=`, not `<`).
    assert_eq!(
        try_u256_to_i128(&U256::from_u128(&env, i128::MAX as u128)),
        Some(i128::MAX)
    );
    let too_big = U256::from_u128(&env, i128::MAX as u128).add(&U256::from_u32(&env, 1));
    assert_eq!(try_u256_to_i128(&too_big), None);
}
