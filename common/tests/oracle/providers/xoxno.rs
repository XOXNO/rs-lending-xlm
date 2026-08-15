extern crate std;

use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contract, contractimpl, Address, Env};

#[contract]
struct StubXoxno;

#[contractimpl]
impl StubXoxno {
    pub fn max_submission_age_seconds(_env: Env) -> u64 {
        120
    }
}

#[test]
#[should_panic]
fn missing_adapter_panics() {
    let env = Env::default();
    let _ = max_submission_age(&env, &Address::generate(&env));
}

#[test]
fn live_adapter_returns_configured_age() {
    let env = Env::default();
    let adapter = env.register(StubXoxno, ());
    assert_eq!(max_submission_age(&env, &adapter), 120);
}
