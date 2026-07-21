//! Invocation-local token accounting.

use soroban_sdk::{panic_with_error, Address, Env, Map};

use crate::errors::Error;

pub(crate) struct Vault<'a> {
    env: &'a Env,
    balances: Map<Address, i128>,
}

impl<'a> Vault<'a> {
    pub fn new(env: &'a Env) -> Self {
        Self {
            env,
            balances: Map::new(env),
        }
    }

    pub fn balance_of(&self, token: &Address) -> i128 {
        self.balances.get(token.clone()).unwrap_or(0)
    }

    pub fn deposit(&mut self, token: &Address, amount: i128) {
        if amount == 0 {
            return;
        }
        if amount < 0 {
            panic_with_error!(self.env, Error::InvalidAmount);
        }
        let current = self.balance_of(token);
        let new = current
            .checked_add(amount)
            .unwrap_or_else(|| panic_with_error!(self.env, Error::IntegerOverflow));
        self.balances.set(token.clone(), new);
    }

    pub fn withdraw(&mut self, token: &Address, amount: i128) {
        if amount == 0 {
            return;
        }
        if amount < 0 {
            panic_with_error!(self.env, Error::InvalidAmount);
        }
        let current = self.balance_of(token);
        if current < amount {
            panic_with_error!(self.env, Error::InvalidAmount);
        }
        self.balances.set(token.clone(), current - amount);
    }
}
