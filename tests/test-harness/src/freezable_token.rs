//! Mock SEP-41 token whose transfers TRAP when the recipient is a configured
//! "blocked" address — modelling a regulated / `AUTH_REQUIRED` / clawback-frozen
//! asset whose issuer withholds authorization from an arbitrary receiver (for
//! example a liquidator that was never authorized, or was frozen/blacklisted).
//!
//! Used to prove that the controller's forced pro-rata liquidation seizure
//! across *every* collateral leg makes an account un-liquidatable when a single
//! leg cannot be delivered to the liquidator. A default Stellar Asset Contract
//! cannot model this in the test host (`set_authorized`/`clawback` require the
//! issuer `AUTH_REVOCABLE`/`AUTH_CLAWBACK_ENABLED` flags, which the test SAC does
//! not carry), so this stand-in reproduces the transfer trap directly.

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, String};

#[contracttype]
enum Key {
    Balance(Address),
    Allowance(Address, Address),
    Blocked,
}

#[contract]
pub struct FreezableToken;

#[contractimpl]
impl FreezableToken {
    /// Blocks (or, with `None`, unblocks) transfers whose recipient is `to`.
    pub fn set_blocked(env: Env, to: Option<Address>) {
        match to {
            Some(addr) => env.storage().instance().set(&Key::Blocked, &addr),
            None => env.storage().instance().remove(&Key::Blocked),
        }
    }

    pub fn mint(env: Env, to: Address, amount: i128) {
        let balance = read_balance(&env, &to);
        write_balance(&env, &to, balance + amount);
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        read_balance(&env, &id)
    }

    pub fn decimals(_env: Env) -> u32 {
        7
    }

    pub fn name(env: Env) -> String {
        String::from_str(&env, "Freezable")
    }

    pub fn symbol(env: Env) -> String {
        String::from_str(&env, "FRZ")
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        do_transfer(&env, &from, &to, amount);
    }

    pub fn transfer_from(env: Env, spender: Address, from: Address, to: Address, amount: i128) {
        spender.require_auth();
        spend_allowance(&env, &from, &spender, amount);
        do_transfer(&env, &from, &to, amount);
    }

    pub fn approve(env: Env, from: Address, spender: Address, amount: i128, _expiration: u32) {
        from.require_auth();
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
        let balance = read_balance(&env, &from);
        write_balance(&env, &from, balance - amount);
    }

    pub fn burn_from(env: Env, spender: Address, from: Address, amount: i128) {
        spender.require_auth();
        spend_allowance(&env, &from, &spender, amount);
        let balance = read_balance(&env, &from);
        write_balance(&env, &from, balance - amount);
    }
}

fn do_transfer(env: &Env, from: &Address, to: &Address, amount: i128) {
    assert!(amount >= 0, "freezable token: negative amount");
    if let Some(blocked) = env.storage().instance().get::<_, Address>(&Key::Blocked) {
        assert!(*to != blocked, "freezable token: recipient not authorized");
    }
    let from_balance = read_balance(env, from);
    assert!(
        from_balance >= amount,
        "freezable token: insufficient balance"
    );
    write_balance(env, from, from_balance - amount);
    let to_balance = read_balance(env, to);
    write_balance(env, to, to_balance + amount);
}

fn spend_allowance(env: &Env, from: &Address, spender: &Address, amount: i128) {
    let key = Key::Allowance(from.clone(), spender.clone());
    let current: i128 = env.storage().instance().get(&key).unwrap_or(0);
    assert!(current >= amount, "freezable token: insufficient allowance");
    env.storage().instance().set(&key, &(current - amount));
}

fn read_balance(env: &Env, addr: &Address) -> i128 {
    env.storage()
        .instance()
        .get(&Key::Balance(addr.clone()))
        .unwrap_or(0)
}

fn write_balance(env: &Env, addr: &Address, amount: i128) {
    env.storage()
        .instance()
        .set(&Key::Balance(addr.clone()), &amount);
}
