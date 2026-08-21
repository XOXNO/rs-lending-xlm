//! Cast confinement on the NFT id domain.
//!
//! An account id is a `u64` in the controller and a `u32` token id on the NFT,
//! so every call that crosses that boundary narrows. The narrowing is the
//! security boundary: an id above `u32::MAX` can never have been minted, so a
//! silent truncation would address a *different, real* token -- burning or
//! renewing someone else's position. Both call sites reject instead, and
//! neither rejection had a test.
extern crate std;

use super::*;

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

use crate::Controller;

/// One past the mintable domain: the smallest id whose `u32` narrowing fails.
const BEYOND_U32: u64 = u32::MAX as u64 + 1;

fn in_controller<T>(env: &Env, body: impl FnOnce() -> T) -> T {
    let admin = Address::generate(env);
    let id = env.register(Controller, (admin,));
    env.as_contract(&id, body)
}

#[test]
#[should_panic(expected = "Error(Contract, #24)")]
fn burning_an_id_outside_the_mintable_domain_is_refused() {
    let env = Env::default();
    let nft = Address::generate(&env);
    // Truncating would make this address token id 0 -- the first NFT ever
    // minted -- and burn it.
    in_controller(&env, || nft_burn_call(&env, &nft, BEYOND_U32));
}

#[test]
#[should_panic(expected = "Error(Contract, #24)")]
fn renewing_an_id_outside_the_mintable_domain_is_refused() {
    let env = Env::default();
    let nft = Address::generate(&env);
    in_controller(&env, || nft_renew_call(&env, &nft, BEYOND_U32));
}

/// The read path narrows too, but fails closed with `None` rather than
/// panicking: an owner lookup is asked all sorts of ids and absence is a
/// legitimate answer. What must not happen is truncation producing an owner.
#[test]
fn an_owner_lookup_outside_the_mintable_domain_reports_no_owner() {
    let env = Env::default();
    let nft = Address::generate(&env);
    // No NFT contract is registered at `nft`, so reaching the client call at
    // all would trap. Returning None before that is the assertion.
    in_controller(&env, || {
        assert_eq!(nft_try_owner_of_call(&env, &nft, BEYOND_U32), None);
    });
}
