//! Tracked-cash gates, credit/debit, and SAC transfers out of the pool.

use common::errors::{CollateralError, GenericError};

use soroban_sdk::{assert_with_error, panic_with_error, token, Address};

use super::Cache;

impl Cache {
    /// Rejects a payout that tracked cash cannot fund.
    ///
    /// # Errors
    /// * `InsufficientLiquidity` — tracked cash cannot cover `amount`.
    pub(crate) fn require_reserves(&self, amount: i128) {
        assert_with_error!(
            self.env,
            self.cash >= amount,
            CollateralError::InsufficientLiquidity
        );
    }

    /// Adds `amount` to tracked cash.
    ///
    /// # Errors
    /// * `MathOverflow` — cash accounting overflows.
    pub(crate) fn credit_cash(&mut self, amount: i128) {
        self.cash = self
            .cash
            .checked_add(amount)
            .unwrap_or_else(|| panic_with_error!(&self.env, GenericError::MathOverflow));
    }

    /// Removes `amount` from tracked cash.
    ///
    /// # Errors
    /// * `MathOverflow` — cash accounting overflows.
    pub(crate) fn debit_cash(&mut self, amount: i128) {
        self.cash = self
            .cash
            .checked_sub(amount)
            .unwrap_or_else(|| panic_with_error!(&self.env, GenericError::MathOverflow));
    }

    /// Sends the market's asset out of the pool. Non-positive amounts are a
    /// no-op so callers need no zero guard.
    pub(crate) fn transfer_out(&self, recipient: &Address, amount: i128) {
        if amount <= 0 {
            return;
        }
        let tok = token::Client::new(&self.env, &self.params.asset_id);
        tok.transfer(&self.env.current_contract_address(), recipient, &amount);
    }
}
