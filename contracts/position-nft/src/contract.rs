//! Lending-position NFT: one token per controller account. Token id ==
//! controller account id. Mint and burn are controller-only; everything else
//! is the stock OpenZeppelin non-fungible interface so external indexers and
//! marketplaces can track positions without protocol-specific tooling.

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, String};
use stellar_tokens::non_fungible::{
    enumerable::{Enumerable, NonFungibleEnumerable},
    sequential, Base, NonFungibleToken,
};

#[contracttype]
pub enum DataKey {
    Controller,
}

fn controller(e: &Env) -> Address {
    // Set in the constructor; the constructor cannot be skipped on Soroban.
    e.storage()
        .instance()
        .get(&DataKey::Controller)
        .expect("controller set at construction")
}

#[contract]
pub struct PositionNft;

#[contractimpl]
impl PositionNft {
    /// `controller` is the only address allowed to mint and burn. Consumes
    /// token id 0 so the first position is id 1 — the controller ABI reserves
    /// account id 0 as the "create new account" sentinel.
    pub fn __constructor(e: &Env, controller: Address, uri: String, name: String, symbol: String) {
        e.storage()
            .instance()
            .set(&DataKey::Controller, &controller);
        Base::set_metadata(e, uri, name, symbol);
        sequential::increment_token_id(e, 1);
    }

    /// Mints the next sequential position token to `to`. Controller-only.
    pub fn mint(e: &Env, to: Address) -> u32 {
        controller(e).require_auth();
        Enumerable::sequential_mint(e, &to)
    }

    /// Burns `token_id` without the holder's authorization. Controller-only.
    ///
    /// Deliberately NOT the OZ `Burnable` extension: `Base::burn` calls
    /// `from.require_auth()`, but the controller must burn when an account
    /// empties through liquidation, where the owner never signed. This
    /// replicates `Enumerable::burn` (v0.7.2) minus that auth:
    /// `Base::update` clears owner/balance/approval, then the enumeration
    /// helper maintains owner enumeration, total supply, and global
    /// enumeration.
    pub fn burn(e: &Env, token_id: u32) {
        controller(e).require_auth();
        let owner = Base::owner_of(e, token_id);
        Base::update(e, Some(&owner), None, token_id);
        stellar_tokens::non_fungible::burnable::emit_burn(e, &owner, token_id);
        Enumerable::remove_from_enumerations(e, &owner, token_id);
    }
}

#[contractimpl(contracttrait)]
impl NonFungibleToken for PositionNft {
    type ContractType = Enumerable;
}

#[contractimpl(contracttrait)]
impl NonFungibleEnumerable for PositionNft {}
