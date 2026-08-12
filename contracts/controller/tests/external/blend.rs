extern crate std;

use super::*;
use crate::Controller;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, Vec};

#[test]
#[should_panic]
fn blend_sweep_all_requires_a_live_pool() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let id = env.register(Controller, (admin,));
    env.as_contract(&id, || {
        let mut assets = Vec::new(&env);
        assets.push_back(Address::generate(&env));
        blend_sweep_all(
            &env,
            &Address::generate(&env),
            &Address::generate(&env),
            &assets,
            &Vec::new(&env),
        );
    });
}

#[test]
#[should_panic]
fn blend_repay_all_requires_a_live_pool() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let id = env.register(Controller, (admin,));
    env.as_contract(&id, || {
        let mut caps = Vec::new(&env);
        caps.push_back((Address::generate(&env), 1i128));
        blend_repay_all(
            &env,
            &Address::generate(&env),
            &Address::generate(&env),
            &caps,
        );
    });
}
