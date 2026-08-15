extern crate std;

use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contract, contractimpl, Address, Env, String, U256};

#[contract]
struct StubRedStone;

#[contractimpl]
impl StubRedStone {
    pub fn read_price_data_for_feed(env: Env, _feed_id: String) -> RedStonePriceData {
        RedStonePriceData {
            price: U256::from_u128(&env, 50_000_000_000),
            package_timestamp: 7,
            write_timestamp: 8,
        }
    }
}

#[test]
fn missing_feed_returns_none() {
    let env = Env::default();
    assert!(read_price_data_uncached(
        &env,
        &Address::generate(&env),
        &String::from_str(&env, "BTC")
    )
    .is_none());
}

#[test]
fn live_feed_returns_price_payload() {
    let env = Env::default();
    let feed = env.register(StubRedStone, ());
    let data =
        read_price_data_uncached(&env, &feed, &String::from_str(&env, "BTC")).expect("price");
    assert_eq!(data.price, U256::from_u128(&env, 50_000_000_000));
    assert_eq!(data.package_timestamp, 7);
    assert_eq!(data.write_timestamp, 8);
}
