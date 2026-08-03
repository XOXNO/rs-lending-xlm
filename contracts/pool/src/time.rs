use common::constants::MS_PER_SECOND;
use common::errors::GenericError;

use soroban_sdk::{panic_with_error, Env};

pub(crate) fn now_ms(env: &Env) -> u64 {
    env.ledger()
        .timestamp()
        .checked_mul(MS_PER_SECOND)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
}
