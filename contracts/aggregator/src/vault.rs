//! In-memory token balance tracker used during `execute_strategy`.
//!
//! Tokens move through the vault in three phases per path:
//!   1. `deposit(token_in, amount_in)` — pulled from `sender` at path start.
//!   2. `withdraw` + `deposit` chain for each hop against the vault.
//!   3. Final output is withdrawn and transferred back to `sender`.
//!
//! We use `soroban_sdk::Map<Address, i128>` because:
//! - It handles hashing/equality via Soroban's host functions.
//! - It lives in `Env` memory for the contract's lifetime — no storage I/O
//!   cost.
//! - Balances are `i128`, matching Soroban SAC token semantics.

use crate::errors::Error;
use soroban_sdk::{panic_with_error, Address, Env, Map};

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

    /// Non-failing lookup — returns 0 for unseen tokens.
    pub fn balance_of(&self, token: &Address) -> i128 {
        self.balances.get(token.clone()).unwrap_or(0)
    }

    /// Add `amount` to the token's running balance. `amount` must be > 0.
    /// Zero deposits are silently no-op'd (vault has no semantic meaning
    /// for a zero credit).
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

    /// Remove `amount` from the token's running balance. Panics with
    /// `InvalidAmount` if the vault doesn't hold enough. Zero withdrawals
    /// are no-ops.
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
