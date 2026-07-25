//! Normalization is one rule per provider shape; `soft` only changes how a
//! failure is reported, never whether the input is acceptable.

extern crate std;

use super::*;
use soroban_sdk::Env;

/// The property that makes one implementation safe: for every input, soft and
/// hard agree on acceptance. Soft returns None exactly when hard would panic.
#[test]
fn soft_and_hard_agree_on_future_timestamp_rejection() {
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

fn reflector_data(env: &Env, price: i128, timestamp: u64) -> ReflectorPriceData {
    let _ = env;
    ReflectorPriceData { price, timestamp }
}
