//! Cash reserve bookkeeping and outbound token transfers.
//!
//! Accounting cash is updated on supply/borrow/repay paths. Live token
//! transfers use the market's `asset_id` SAC client.

use common::errors::{CollateralError, GenericError};

use soroban_sdk::{assert_with_error, panic_with_error, token, Address};

use super::Cache;

impl Cache {
    /// Panic if cash reserves are below `amount`.
    pub(crate) fn require_reserves(&self, amount: i128) {
        assert_with_error!(
            self.env,
            self.cash >= amount,
            CollateralError::InsufficientLiquidity
        );
    }

    /// Increase accounting cash by `amount` (checked add).
    pub(crate) fn credit_cash(&mut self, amount: i128) {
        self.cash = self
            .cash
            .checked_add(amount)
            .unwrap_or_else(|| panic_with_error!(&self.env, GenericError::MathOverflow));
    }

    /// Decrease accounting cash by `amount` (checked sub).
    pub(crate) fn debit_cash(&mut self, amount: i128) {
        self.cash = self
            .cash
            .checked_sub(amount)
            .unwrap_or_else(|| panic_with_error!(&self.env, GenericError::MathOverflow));
    }

    /// Transfer `amount` of the market asset from the pool to `recipient`.
    ///
    /// No-op when `amount <= 0`. Does not adjust accounting cash; callers debit
    /// cash separately when needed.
    pub(crate) fn transfer_out(&self, recipient: &Address, amount: i128) {
        if amount <= 0 {
            return;
        }
        let tok = token::Client::new(&self.env, &self.params.asset_id);
        tok.transfer(&self.env.current_contract_address(), recipient, &amount);
    }
}
