//! Ledger time helpers for interest accrual.
//!
//! Market timestamps are stored in **milliseconds**. Soroban ledger time is
//! seconds, so this module multiplies by [`MS_PER_SECOND`].

use common::constants::MS_PER_SECOND;
use common::errors::GenericError;

use soroban_sdk::{panic_with_error, Env};

/// Current ledger timestamp converted to milliseconds.
///
/// Panics with [`GenericError::MathOverflow`] if `timestamp * MS_PER_SECOND`
/// would overflow `u64`.
pub(crate) fn now_ms(env: &Env) -> u64 {
    env.ledger()
        .timestamp()
        .checked_mul(MS_PER_SECOND)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
}
