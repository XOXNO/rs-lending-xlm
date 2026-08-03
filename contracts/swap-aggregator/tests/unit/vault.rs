use crate::errors::Error;
use crate::vault::Vault;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

#[test]
fn vault_accounting_unit() {
    let env = Env::default();
    let token = Address::generate(&env);
    let mut v = Vault::new(&env);
    assert_eq!(v.balance_of(&token), 0);
    v.deposit(&token, 0);
    assert_eq!(v.balance_of(&token), 0);
    v.deposit(&token, 100);
    assert_eq!(v.balance_of(&token), 100);
    v.withdraw(&token, 0);
    assert_eq!(v.balance_of(&token), 100);
    v.withdraw(&token, 40);
    assert_eq!(v.balance_of(&token), 60);
}

#[test]
fn vault_deposit_negative_returns_invalid_amount() {
    let env = Env::default();
    let token = Address::generate(&env);
    assert_eq!(
        Vault::new(&env).try_deposit(&token, -1),
        Err(Error::InvalidAmount)
    );
}

#[test]
fn vault_withdraw_negative_returns_invalid_amount() {
    let env = Env::default();
    let token = Address::generate(&env);
    assert_eq!(
        Vault::new(&env).try_withdraw(&token, -1),
        Err(Error::InvalidAmount)
    );
}

#[test]
fn vault_withdraw_overdraw_returns_invalid_amount() {
    let env = Env::default();
    let token = Address::generate(&env);
    let mut v = Vault::new(&env);
    v.deposit(&token, 10);
    assert_eq!(v.try_withdraw(&token, 20), Err(Error::InvalidAmount));
}
