//! Live stubs so fail-open Aquarius helpers cannot hide as `None`/`false`.
extern crate std;

use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contract, contractimpl, symbol_short, Address, Env, Symbol, Vec};

#[contract]
struct StubPool;

#[contractimpl]
impl StubPool {
    pub fn __constructor(
        env: Env,
        kind: Symbol,
        share: Address,
        token_a: Address,
        token_b: Address,
    ) {
        env.storage().instance().set(&symbol_short!("kind"), &kind);
        env.storage()
            .instance()
            .set(&symbol_short!("share"), &share);
        env.storage().instance().set(&symbol_short!("ta"), &token_a);
        env.storage().instance().set(&symbol_short!("tb"), &token_b);
    }

    pub fn get_total_shares(_env: Env) -> u128 {
        1_000
    }

    pub fn get_reserves(_env: Env) -> Vec<u128> {
        Vec::from_array(&_env, [11u128, 22u128])
    }

    pub fn get_tokens(env: Env) -> Vec<Address> {
        let ta: Address = env.storage().instance().get(&symbol_short!("ta")).unwrap();
        let tb: Address = env.storage().instance().get(&symbol_short!("tb")).unwrap();
        Vec::from_array(&env, [ta, tb])
    }

    pub fn pool_type(env: Env) -> Symbol {
        env.storage()
            .instance()
            .get(&symbol_short!("kind"))
            .unwrap()
    }

    pub fn share_id(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&symbol_short!("share"))
            .unwrap()
    }

    pub fn a(_env: Env) -> u128 {
        1_500
    }
}

fn register_pool(env: &Env, kind: &str) -> (Address, Address, Address, Address) {
    let share = Address::generate(env);
    let token_a = Address::generate(env);
    let token_b = Address::generate(env);
    let pool = env.register(
        StubPool,
        (
            Symbol::new(env, kind),
            share.clone(),
            token_a.clone(),
            token_b.clone(),
        ),
    );
    (pool, share, token_a, token_b)
}

#[test]
fn missing_pool_helpers_fail_open() {
    let env = Env::default();
    let missing = Address::generate(&env);
    assert_eq!(aquarius_pool_reserves_call(&env, &missing), None);
    assert_eq!(aquarius_amp_call(&env, &missing), None);
    assert!(!aquarius_is_stable_call(&env, &missing));
    assert!(!aquarius_is_constant_product_call(&env, &missing));
    assert_eq!(aquarius_get_tokens_call(&env, &missing), None);
    assert_eq!(aquarius_share_id_call(&env, &missing), None);
    assert_eq!(aquarius_total_shares_call(&env, &missing), None);
}

#[test]
fn pool_helpers_return_live_metadata() {
    let env = Env::default();
    let (stable, share, token_a, token_b) = register_pool(&env, "stable");
    let (cp, _, _, _) = register_pool(&env, "constant_product");

    assert_eq!(aquarius_pool_reserves_call(&env, &stable), Some((11, 22)));
    assert_eq!(aquarius_amp_call(&env, &stable), Some(1_500));
    assert!(aquarius_is_stable_call(&env, &stable));
    assert!(!aquarius_is_constant_product_call(&env, &stable));
    assert!(aquarius_is_constant_product_call(&env, &cp));
    assert!(!aquarius_is_stable_call(&env, &cp));
    assert_eq!(aquarius_total_shares_call(&env, &stable), Some(1_000));
    assert_eq!(aquarius_share_id_call(&env, &stable), Some(share));
    let tokens = aquarius_get_tokens_call(&env, &stable).expect("tokens");
    assert_eq!(tokens.len(), 2);
    assert_eq!(tokens.get_unchecked(0), token_a);
    assert_eq!(tokens.get_unchecked(1), token_b);
}

#[contract]
struct StubZeroAmp;

#[contractimpl]
impl StubZeroAmp {
    pub fn a(_env: Env) -> u128 {
        0
    }
}

#[test]
fn amp_zero_is_rejected() {
    let env = Env::default();
    let pool = env.register(StubZeroAmp, ());
    assert_eq!(aquarius_amp_call(&env, &pool), None);
}
