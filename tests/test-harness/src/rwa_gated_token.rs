//! Model of the Asterizm RWA token gate (`asterizm-rwa-example`) that Liqvid
//! deal tokens use: while the gate is active, `mint`, `transfer`,
//! `transfer_from`, `burn` and `burn_from` require every holder they touch
//! (`from` and `to`, never the spender) to be allowlisted and not frozen,
//! zero amounts included. `set_metadata` rewrites `decimals` without
//! rescaling balances. Issuer-only methods skip authorization so tests can
//! drive them directly.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, Address, Env, String,
    Vec,
};

#[contracterror]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u32)]
pub enum RwaTokenError {
    AccountNotAllowed = 7,
    AccountFrozen = 8,
}

#[contracttype]
enum Key {
    Balance(Address),
    Allowance(Address, Address),
    Allowlisted(Address),
    Frozen(Address),
    GateActive,
    Decimals,
}

#[contract]
pub struct RwaGatedToken;

#[contractimpl]
impl RwaGatedToken {
    pub fn set_gate_active(env: Env, active: bool) {
        env.storage().instance().set(&Key::GateActive, &active);
    }

    pub fn add_to_allowlist(env: Env, accounts: Vec<Address>) {
        for account in accounts.iter() {
            env.storage()
                .instance()
                .set(&Key::Allowlisted(account), &true);
        }
    }

    pub fn remove_from_allowlist(env: Env, accounts: Vec<Address>) {
        for account in accounts.iter() {
            env.storage().instance().remove(&Key::Allowlisted(account));
        }
    }

    pub fn freeze(env: Env, account: Address) {
        env.storage().instance().set(&Key::Frozen(account), &true);
    }

    pub fn unfreeze(env: Env, account: Address) {
        env.storage().instance().remove(&Key::Frozen(account));
    }

    pub fn set_metadata(env: Env, decimal: u32, _name: String, _symbol: String) {
        assert!(decimal <= 18, "Decimal must not be greater than 18");
        env.storage().instance().set(&Key::Decimals, &decimal);
    }

    pub fn is_allowlisted(env: Env, account: Address) -> bool {
        env.storage().instance().has(&Key::Allowlisted(account))
    }

    pub fn is_frozen(env: Env, account: Address) -> bool {
        env.storage().instance().has(&Key::Frozen(account))
    }

    pub fn mint(env: Env, to: Address, amount: i128) {
        require_nonnegative(amount);
        ensure_can_hold(&env, &to);
        write_balance(&env, &to, read_balance(&env, &to) + amount);
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        read_balance(&env, &id)
    }

    pub fn decimals(env: Env) -> u32 {
        env.storage().instance().get(&Key::Decimals).unwrap_or(0)
    }

    pub fn name(env: Env) -> String {
        String::from_str(&env, "Liqvid Deal Share")
    }

    pub fn symbol(env: Env) -> String {
        String::from_str(&env, "LIQVID")
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        require_nonnegative(amount);
        ensure_can_hold(&env, &from);
        ensure_can_hold(&env, &to);
        move_balance(&env, &from, &to, amount);
    }

    pub fn transfer_from(env: Env, spender: Address, from: Address, to: Address, amount: i128) {
        spender.require_auth();
        require_nonnegative(amount);
        ensure_can_hold(&env, &from);
        ensure_can_hold(&env, &to);
        spend_allowance(&env, &from, &spender, amount);
        move_balance(&env, &from, &to, amount);
    }

    pub fn approve(env: Env, from: Address, spender: Address, amount: i128, _expiration: u32) {
        from.require_auth();
        require_nonnegative(amount);
        env.storage()
            .instance()
            .set(&Key::Allowance(from, spender), &amount);
    }

    pub fn allowance(env: Env, from: Address, spender: Address) -> i128 {
        env.storage()
            .instance()
            .get(&Key::Allowance(from, spender))
            .unwrap_or(0)
    }

    pub fn burn(env: Env, from: Address, amount: i128) {
        from.require_auth();
        require_nonnegative(amount);
        ensure_can_hold(&env, &from);
        spend_balance(&env, &from, amount);
    }

    pub fn burn_from(env: Env, spender: Address, from: Address, amount: i128) {
        spender.require_auth();
        require_nonnegative(amount);
        ensure_can_hold(&env, &from);
        spend_allowance(&env, &from, &spender, amount);
        spend_balance(&env, &from, amount);
    }
}

fn require_nonnegative(amount: i128) {
    assert!(amount >= 0, "negative amount is not allowed: {amount}");
}

fn ensure_can_hold(env: &Env, holder: &Address) {
    let active: bool = env
        .storage()
        .instance()
        .get(&Key::GateActive)
        .unwrap_or(false);
    if !active {
        return;
    }
    if !env
        .storage()
        .instance()
        .has(&Key::Allowlisted(holder.clone()))
    {
        panic_with_error!(env, RwaTokenError::AccountNotAllowed);
    }
    if env.storage().instance().has(&Key::Frozen(holder.clone())) {
        panic_with_error!(env, RwaTokenError::AccountFrozen);
    }
}

fn read_balance(env: &Env, id: &Address) -> i128 {
    env.storage()
        .instance()
        .get(&Key::Balance(id.clone()))
        .unwrap_or(0)
}

fn write_balance(env: &Env, id: &Address, amount: i128) {
    env.storage()
        .instance()
        .set(&Key::Balance(id.clone()), &amount);
}

fn spend_balance(env: &Env, from: &Address, amount: i128) {
    let balance = read_balance(env, from);
    assert!(balance >= amount, "insufficient balance");
    write_balance(env, from, balance - amount);
}

fn move_balance(env: &Env, from: &Address, to: &Address, amount: i128) {
    spend_balance(env, from, amount);
    write_balance(env, to, read_balance(env, to) + amount);
}

fn spend_allowance(env: &Env, from: &Address, spender: &Address, amount: i128) {
    let key = Key::Allowance(from.clone(), spender.clone());
    let allowance: i128 = env.storage().instance().get(&key).unwrap_or(0);
    assert!(allowance >= amount, "insufficient allowance");
    env.storage().instance().set(&key, &(allowance - amount));
}
