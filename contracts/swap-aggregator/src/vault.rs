//! Invocation-local token ledger for one `execute_strategy` call.
//!
//! Tracks spendable balances and lifetime credits (deposits only). Credits
//! drive residual allowance; withdrawals do not reduce credited amounts.

use soroban_sdk::{panic_with_error, Address, Env, Map, Vec};

use crate::errors::Error;

/// In-memory balances for tokens held by the router during a strategy.
pub(crate) struct Vault<'a> {
    env: &'a Env,
    balances: Map<Address, i128>,
    /// Cumulative deposits per token (never decreased by withdraw).
    credited: Map<Address, i128>,
}

impl<'a> Vault<'a> {
    /// Creates an empty vault bound to `env`.
    pub fn new(env: &'a Env) -> Self {
        Self {
            env,
            balances: Map::new(env),
            credited: Map::new(env),
        }
    }

    /// Returns the current tracked balance for `token`.
    pub fn balance_of(&self, token: &Address) -> i128 {
        self.balances.get(token.clone()).unwrap_or(0)
    }

    /// Returns the tokens with a tracked balance entry; the balance itself may be zero.
    pub fn tokens(&self) -> Vec<Address> {
        self.balances.keys()
    }

    /// Returns the lifetime credited amount for `token` (deposits only).
    pub fn credited_of(&self, token: &Address) -> i128 {
        self.credited.get(token.clone()).unwrap_or(0)
    }

    /// Credits `amount` to `token`, updating both the tracked balance and the lifetime
    /// credited total. A zero amount is a no-op. Panics with `Error::InvalidAmount` for a
    /// negative amount and `Error::IntegerOverflow` if either total would overflow.
    pub fn deposit(&mut self, token: &Address, amount: i128) {
        if amount == 0 {
            return;
        }
        let env = self.env;
        if amount < 0 {
            panic_with_error!(env, Error::InvalidAmount);
        }
        let current = self.balance_of(token);
        let new = current
            .checked_add(amount)
            .unwrap_or_else(|| panic_with_error!(env, Error::IntegerOverflow));
        self.balances.set(token.clone(), new);
        let credited = self
            .credited_of(token)
            .checked_add(amount)
            .unwrap_or_else(|| panic_with_error!(env, Error::IntegerOverflow));
        self.credited.set(token.clone(), credited);
    }

    /// Debits `amount` from `token`'s tracked balance. A zero amount is a no-op. Panics with
    /// `Error::InvalidAmount` for a negative amount or if `amount` exceeds the current
    /// balance.
    pub fn withdraw(&mut self, token: &Address, amount: i128) {
        if amount == 0 {
            return;
        }
        let env = self.env;
        if amount < 0 {
            panic_with_error!(env, Error::InvalidAmount);
        }
        let current = self.balance_of(token);
        if current < amount {
            panic_with_error!(env, Error::InvalidAmount);
        }
        self.balances.set(token.clone(), current - amount);
    }
}
