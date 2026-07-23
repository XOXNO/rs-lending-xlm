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

    pub fn try_deposit(&mut self, token: &Address, amount: i128) -> Result<(), Error> {
        if amount == 0 {
            return Ok(());
        }
        if amount < 0 {
            return Err(Error::InvalidAmount);
        }
        let current = self.balance_of(token);
        let new = current.checked_add(amount).ok_or(Error::IntegerOverflow)?;
        self.balances.set(token.clone(), new);
        Ok(())
    }

    pub fn deposit(&mut self, token: &Address, amount: i128) {
        if let Err(err) = self.try_deposit(token, amount) {
            panic_with_error!(self.env, err);
        }
    }

    pub fn try_withdraw(&mut self, token: &Address, amount: i128) -> Result<(), Error> {
        if amount == 0 {
            return Ok(());
        }
        if amount < 0 {
            return Err(Error::InvalidAmount);
        }
        let current = self.balance_of(token);
        if current < amount {
            return Err(Error::InvalidAmount);
        }
        self.balances.set(token.clone(), current - amount);
        Ok(())
    }

    pub fn withdraw(&mut self, token: &Address, amount: i128) {
        if let Err(err) = self.try_withdraw(token, amount) {
            panic_with_error!(self.env, err);
        }
    }
}
