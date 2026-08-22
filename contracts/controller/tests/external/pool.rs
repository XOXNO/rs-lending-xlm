extern crate std;

use super::*;
use crate::Controller;
use common::types::HubAssetKey;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec, Address, Bytes, BytesN, Env, Vec};

fn dummy_hub(env: &Env) -> HubAssetKey {
    HubAssetKey {
        hub_id: 1,
        asset: Address::generate(env),
    }
}

#[test]
#[should_panic(expected = "Error(Storage, MissingValue)")]
fn pool_wrappers_require_a_live_pool() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let id = env.register(Controller, (admin,));
    let missing = Address::generate(&env);
    env.as_contract(&id, || {
        let _ = pool_flash_loan_call(
            &env,
            &missing,
            &dummy_hub(&env),
            &Address::generate(&env),
            &Address::generate(&env),
            1,
            &Bytes::new(&env),
        );
    });
}

#[test]
#[should_panic(expected = "Error(Storage, MissingValue)")]
fn pool_update_indexes_requires_a_live_pool() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let id = env.register(Controller, (admin,));
    env.as_contract(&id, || {
        pool_update_indexes_call(&env, &Address::generate(&env), &vec![&env, dummy_hub(&env)]);
    });
}

#[test]
#[should_panic(expected = "Error(Storage, MissingValue)")]
fn pool_upgrade_requires_a_live_pool() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let id = env.register(Controller, (admin,));
    env.as_contract(&id, || {
        pool_upgrade_call(
            &env,
            &Address::generate(&env),
            &BytesN::from_array(&env, &[0u8; 32]),
        );
    });
}

#[test]
#[should_panic(expected = "Error(Storage, MissingValue)")]
fn pool_create_market_requires_a_live_pool() {
    use common::types::MarketParamsRaw;
    let env = Env::default();
    let admin = Address::generate(&env);
    let id = env.register(Controller, (admin,));
    let asset = Address::generate(&env);
    env.as_contract(&id, || {
        pool_create_market_call(
            &env,
            &Address::generate(&env),
            1,
            &MarketParamsRaw {
                max_borrow_rate: 0,
                base_borrow_rate: 0,
                slope1: 0,
                slope2: 0,
                slope3: 0,
                mid_utilization: 0,
                optimal_utilization: 0,
                max_utilization: 0,
                reserve_factor: 0,
                is_flashloanable: false,
                flashloan_fee: 0,
                asset_id: asset,
                asset_decimals: 7,
            },
        );
    });
}

#[test]
#[should_panic(expected = "Error(Storage, MissingValue)")]
fn pool_supply_requires_a_live_pool() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let id = env.register(Controller, (admin,));
    env.as_contract(&id, || {
        let _ = pool_supply_call(&env, &Address::generate(&env), &Vec::new(&env));
    });
}
