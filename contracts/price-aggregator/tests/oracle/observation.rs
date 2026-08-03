extern crate std;

use super::*;
use soroban_sdk::{Env, U256};

#[test]
fn reflector_rejects_future_timestamp() {
    let env = Env::default();
    let data = reflector_data(&env, 100, 2_000);
    assert!(OracleObservation::from_reflector(1_000, &data, 7).is_none());
}

#[test]
fn reflector_accepts_fresh_positive_price() {
    let env = Env::default();
    let data = reflector_data(&env, 100, 900);
    let obs = OracleObservation::from_reflector(1_000, &data, 7)
        .expect("fresh positive price is acceptable");
    assert!(obs.price_wad > 0);
    assert_eq!(obs.timestamp, 900);
}

#[test]
fn reflector_rejects_non_positive_price() {
    let env = Env::default();
    assert!(OracleObservation::from_reflector(1_000, &reflector_data(&env, 0, 900), 7).is_none());
    assert!(
        OracleObservation::from_reflector(1_000, &reflector_data(&env, -100, 900), 7).is_none()
    );
}

#[test]
fn multi_feed_accepts_fresh_positive_price() {
    let env = Env::default();

    let data = multi_feed_data(&env, 12_345, 900_000, 950_000);
    let obs = OracleObservation::from_multi_feed(1_000, &data, 8)
        .expect("fresh positive price is acceptable");
    assert_eq!(obs.timestamp, 900);
}

#[test]
fn multi_feed_rejects_future_package_timestamp() {
    let env = Env::default();
    let data = multi_feed_data(&env, 100, 2_000_000, 900_000);
    assert!(OracleObservation::from_multi_feed(1_000, &data, 8).is_none());
}

#[test]
fn multi_feed_rejects_future_write_timestamp() {
    let env = Env::default();
    let data = multi_feed_data(&env, 100, 900_000, 2_000_000);
    assert!(OracleObservation::from_multi_feed(1_000, &data, 8).is_none());
}

#[test]
fn multi_feed_rejects_non_positive_price() {
    let env = Env::default();
    let data = multi_feed_data(&env, 0, 900_000, 950_000);
    assert!(OracleObservation::from_multi_feed(1_000, &data, 8).is_none());
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
