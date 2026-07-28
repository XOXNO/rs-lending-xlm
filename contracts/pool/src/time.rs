//! Ledger clock. Interest accrual works in milliseconds; the ledger reports
//! seconds, so every timestamp entering pool state passes through here — a unit
//! slip here would scale all interest by 1000.

use common::constants::MS_PER_SECOND;
use common::errors::GenericError;

use soroban_sdk::{panic_with_error, Env};

/// Returns the current ledger time in milliseconds.
pub(crate) fn now_ms(env: &Env) -> u64 {
    env.ledger()
        .timestamp()
        .checked_mul(MS_PER_SECOND)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
}
