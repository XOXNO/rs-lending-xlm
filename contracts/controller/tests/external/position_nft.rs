//! Cast confinement on the NFT id domain.
//!
//! An account id is a `u64` in the controller and a `u32` token id on the NFT.
//! An id above `u32::MAX` was never minted, and truncating it can address a
//! different, real token. Burn and renew reject such ids with `AccountNotFound`.
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
    in_controller(&env, || nft_burn_call(&env, &nft, BEYOND_U32));
}

#[test]
#[should_panic(expected = "Error(Contract, #24)")]
fn renewing_an_id_outside_the_mintable_domain_is_refused() {
    let env = Env::default();
    let nft = Address::generate(&env);
    in_controller(&env, || nft_renew_call(&env, &nft, BEYOND_U32));
}

/// The owner lookup returns `None` for an id outside the `u32` domain instead
/// of panicking or returning a truncated id's owner.
#[test]
fn an_owner_lookup_outside_the_mintable_domain_reports_no_owner() {
    let env = Env::default();
    let nft = Address::generate(&env);
    // No NFT contract is registered at `nft`.
    in_controller(&env, || {
        assert_eq!(nft_try_owner_of_call(&env, &nft, BEYOND_U32), None);
    });
}
