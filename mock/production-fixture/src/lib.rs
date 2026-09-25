#![no_std]
//! Test-only decimal token and configurable Aquarius read surface. Not a venue.
use soroban_sdk::{
    contract, contractimpl, contracttype, Address, Env, MuxedAddress, String, Symbol, Vec,
};
use stellar_tokens::fungible::{Base, FungibleToken};

#[contracttype]
#[derive(Clone)]
pub struct PoolSnapshot {
    pub tokens: Vec<Address>,
    pub reserves: Vec<u128>,
    pub shares: u128,
    pub share_token: Address,
    pub stable: bool,
    pub amplification: u128,
}

#[contracttype]
enum Key {
    Admin,
    Pool,
}

#[contract]
pub struct ProductionFixture;

#[contractimpl(contracttrait)]
impl FungibleToken for ProductionFixture {
    type ContractType = Base;

    // Keep the generated CLI flag compatible with SAC balance --id.
    fn balance(e: &Env, id: Address) -> i128 {
        Base::balance(e, &id)
    }
}

#[contractimpl]
impl ProductionFixture {
    pub fn __constructor(e: &Env, admin: Address, decimals: u32, symbol: String) {
        assert!(decimals <= 18);
        e.storage().instance().set(&Key::Admin, &admin);
        Base::set_metadata(e, decimals, symbol.clone(), symbol);
    }
    pub fn mint(e: &Env, to: Address, amount: i128) {
        let admin: Address = e.storage().instance().get(&Key::Admin).unwrap();
        admin.require_auth();
        Base::mint(e, &to, amount);
    }
    pub fn configure_pool(e: &Env, snapshot: PoolSnapshot) {
        let admin: Address = e.storage().instance().get(&Key::Admin).unwrap();
        admin.require_auth();
        assert!(snapshot.tokens.len() == 2 && snapshot.reserves.len() == 2);
        assert!(snapshot.shares > 0 && snapshot.amplification > 0);
        e.storage().instance().set(&Key::Pool, &snapshot);
    }
    pub fn get_tokens(e: &Env) -> Vec<Address> {
        snapshot(e).tokens
    }
    pub fn get_reserves(e: &Env) -> Vec<u128> {
        snapshot(e).reserves
    }
    pub fn get_total_shares(e: &Env) -> u128 {
        snapshot(e).shares
    }
    pub fn share_id(e: &Env) -> Address {
        snapshot(e).share_token
    }
    pub fn a(e: &Env) -> u128 {
        snapshot(e).amplification
    }
    pub fn pool_type(e: &Env) -> Symbol {
        Symbol::new(
            e,
            if snapshot(e).stable {
                "stable"
            } else {
                "constant_product"
            },
        )
    }
}
fn snapshot(e: &Env) -> PoolSnapshot {
    e.storage().instance().get(&Key::Pool).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::xdr::{Limits, ReadXdr, ScSpecEntry};

    #[test]
    fn balance_spec_accepts_standard_cli_id_argument() {
        let entry =
            ScSpecEntry::from_xdr(ProductionFixture::spec_xdr_balance(), Limits::none()).unwrap();
        let ScSpecEntry::FunctionV0(function) = entry else {
            panic!("expected balance function spec");
        };
        assert_eq!(function.inputs.len(), 1);
        assert_eq!(function.inputs[0].name.to_utf8_string().unwrap(), "id");
    }

    #[test]
    fn preserves_eighteen_decimal_amounts_and_enforces_minter_auth() {
        let e = Env::default();
        let admin = Address::generate(&e);
        let user = Address::generate(&e);
        let id = e.register(
            ProductionFixture,
            (&admin, 18u32, String::from_str(&e, "FIX")),
        );
        let c = ProductionFixtureClient::new(&e, &id);
        assert!(c.try_mint(&user, &1).is_err());
        e.mock_all_auths();
        let amount = 1_000_000_000_000_000_001i128;
        c.mint(&user, &amount);
        assert_eq!(c.balance(&user), amount);
        assert_eq!(c.decimals(), 18);
        c.transfer(&user, &admin, &amount);
        assert_eq!(c.balance(&admin), amount);
        assert_eq!(c.balance(&user), 0);
    }
}
