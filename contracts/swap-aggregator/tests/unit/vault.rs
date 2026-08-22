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
#[should_panic(expected = "Error(Contract, #3)")]
fn vault_deposit_negative_panics_invalid_amount() {
    let env = Env::default();
    let token = Address::generate(&env);
    Vault::new(&env).deposit(&token, -1);
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn vault_withdraw_negative_panics_invalid_amount() {
    let env = Env::default();
    let token = Address::generate(&env);
    Vault::new(&env).withdraw(&token, -1);
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn vault_withdraw_overdraw_panics_invalid_amount() {
    let env = Env::default();
    let token = Address::generate(&env);
    let mut v = Vault::new(&env);
    v.deposit(&token, 10);
    v.withdraw(&token, 20);
}

#[test]
fn credited_accumulates_and_ignores_withdrawals() {
    let env = Env::default();
    let token = Address::generate(&env);
    let other = Address::generate(&env);
    let mut vault = Vault::new(&env);

    assert_eq!(vault.credited_of(&token), 0, "unseen token starts at zero");

    vault.deposit(&token, 400);
    assert_eq!(vault.credited_of(&token), 400);

    vault.deposit(&token, 600);
    assert_eq!(vault.credited_of(&token), 1_000);
    assert_eq!(vault.balance_of(&token), 1_000);

    vault.withdraw(&token, 900);
    assert_eq!(vault.balance_of(&token), 100);
    assert_eq!(vault.credited_of(&token), 1_000);

    vault.withdraw(&token, 100);
    assert_eq!(vault.balance_of(&token), 0);
    assert_eq!(vault.credited_of(&token), 1_000);

    assert_eq!(vault.credited_of(&other), 0);
}

#[test]
fn credited_ignores_zero_deposits() {
    let env = Env::default();
    let token = Address::generate(&env);
    let mut vault = Vault::new(&env);

    vault.deposit(&token, 0);
    assert_eq!(vault.credited_of(&token), 0);

    vault.deposit(&token, 7);
    vault.deposit(&token, 0);
    assert_eq!(vault.credited_of(&token), 7);
}
