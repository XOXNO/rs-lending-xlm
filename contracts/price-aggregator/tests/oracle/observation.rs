//! Normalization is one rule per provider shape; `soft` only changes how a
//! failure is reported, never whether the input is acceptable.

extern crate std;

use super::*;
use soroban_sdk::{Env, U256};

/// The property that makes one implementation safe: for every input, soft and
/// hard agree on acceptance. Soft returns None exactly when hard would panic.
#[test]
fn reflector_soft_rejects_future_timestamp_by_returning_none() {
    let env = Env::default();
    let now = 1_000u64;
    let future = 2_000u64;

    let data = reflector_data(&env, 100, future);
    // soft: rejected without panicking
    assert!(OracleObservation::from_reflector(&env, now, &data, 7, true).is_none());
}

#[test]
#[should_panic]
fn hard_rejects_future_timestamp_by_panicking() {
    let env = Env::default();
    let data = reflector_data(&env, 100, 2_000);
    let _ = OracleObservation::from_reflector(&env, 1_000, &data, 7, false);
}

#[test]
fn accepts_a_fresh_positive_price_under_both_disciplines() {
    let env = Env::default();
    let data = reflector_data(&env, 100, 900);
    let soft = OracleObservation::from_reflector(&env, 1_000, &data, 7, true)
        .expect("fresh positive price is acceptable");
    let hard = OracleObservation::from_reflector(&env, 1_000, &data, 7, false)
        .expect("fresh positive price is acceptable");
    assert_eq!(soft.price_wad, hard.price_wad);
    assert_eq!(soft.timestamp(), hard.timestamp());
}

#[test]
fn reflector_soft_rejects_non_positive_price_by_returning_none() {
    let env = Env::default();
    // Fresh timestamp so the price check, not the timestamp check, is what
    // rejects the payload.
    assert!(
        OracleObservation::from_reflector(&env, 1_000, &reflector_data(&env, 0, 900), 7, true)
            .is_none()
    );
    assert!(OracleObservation::from_reflector(
        &env,
        1_000,
        &reflector_data(&env, -100, 900),
        7,
        true
    )
    .is_none());
}

#[test]
#[should_panic]
fn reflector_hard_rejects_non_positive_price_by_panicking() {
    let env = Env::default();
    let data = reflector_data(&env, 0, 900);
    let _ = OracleObservation::from_reflector(&env, 1_000, &data, 7, false);
}

#[test]
fn multi_feed_accepts_a_fresh_positive_price_under_both_disciplines() {
    let env = Env::default();
    // Distinct package/write timestamps so a swap between `observed_at` and
    // `published_at` would be caught.
    let data = multi_feed_data(&env, 12_345, 900_000, 950_000);
    let soft = OracleObservation::from_multi_feed(&env, 1_000, &data, 8, true)
        .expect("fresh positive price is acceptable");
    let hard = OracleObservation::from_multi_feed(&env, 1_000, &data, 8, false)
        .expect("fresh positive price is acceptable");
    assert_eq!(soft.price_wad, hard.price_wad);
    assert_eq!(soft.observed_at, hard.observed_at);
    assert_eq!(soft.published_at, hard.published_at);
    assert_eq!(soft.observed_at, 950);
    assert_eq!(soft.published_at, Some(900));
}

#[test]
fn multi_feed_soft_rejects_future_package_timestamp_by_returning_none() {
    let env = Env::default();
    // package future, write fresh.
    let data = multi_feed_data(&env, 100, 2_000_000, 900_000);
    assert!(OracleObservation::from_multi_feed(&env, 1_000, &data, 8, true).is_none());
}

#[test]
#[should_panic]
fn multi_feed_hard_rejects_future_package_timestamp_by_panicking() {
    let env = Env::default();
    let data = multi_feed_data(&env, 100, 2_000_000, 900_000);
    let _ = OracleObservation::from_multi_feed(&env, 1_000, &data, 8, false);
}

#[test]
fn multi_feed_soft_rejects_future_write_timestamp_by_returning_none() {
    let env = Env::default();
    // package fresh, write future: exercises the write leg of the OR
    // independently of the package leg.
    let data = multi_feed_data(&env, 100, 900_000, 2_000_000);
    assert!(OracleObservation::from_multi_feed(&env, 1_000, &data, 8, true).is_none());
}

#[test]
#[should_panic]
fn multi_feed_hard_rejects_future_write_timestamp_by_panicking() {
    let env = Env::default();
    // package fresh (first panic site passes), write future: exercises the
    // second, sequential panic site.
    let data = multi_feed_data(&env, 100, 900_000, 2_000_000);
    let _ = OracleObservation::from_multi_feed(&env, 1_000, &data, 8, false);
}

#[test]
fn multi_feed_soft_rejects_non_positive_price_by_returning_none() {
    let env = Env::default();
    // U256 is unsigned, so the only non-positive value it can carry is zero.
    let data = multi_feed_data(&env, 0, 900_000, 950_000);
    assert!(OracleObservation::from_multi_feed(&env, 1_000, &data, 8, true).is_none());
}

#[test]
#[should_panic]
fn multi_feed_hard_rejects_non_positive_price_by_panicking() {
    let env = Env::default();
    let data = multi_feed_data(&env, 0, 900_000, 950_000);
    let _ = OracleObservation::from_multi_feed(&env, 1_000, &data, 8, false);
}

fn reflector_data(env: &Env, price: i128, timestamp: u64) -> ReflectorPriceData {
    let _ = env;
    ReflectorPriceData { price, timestamp }
}

fn multi_feed_data(
    env: &Env,
    price: u128,
    package_timestamp: u64,
    write_timestamp: u64,
) -> RedStonePriceData {
    RedStonePriceData {
        price: U256::from_u128(env, price),
        package_timestamp,
        write_timestamp,
    }
}
