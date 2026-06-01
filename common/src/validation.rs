//! Cross-contract guard checks shared by the pool and controller.
//!
//! Each guard panics with a stable protocol error so both contracts report
//! identical error codes for the same malformed input.

use crate::errors::{FlashLoanError, GenericError};
use soroban_sdk::{assert_with_error, Address, Env, Executable};

/// Rejects a non-positive amount (`amount <= 0`) with `AmountMustBePositive`.
pub fn require_positive_amount(env: &Env, amount: i128) {
    assert_with_error!(env, amount > 0, GenericError::AmountMustBePositive);
}

/// Rejects a negative amount (`amount < 0`) with `AmountMustBePositive`. Zero is
/// accepted for flows where it carries a sentinel meaning (withdraw-all, zero
/// fee, zero rewards).
pub fn require_nonneg_amount(env: &Env, amount: i128) {
    assert_with_error!(env, amount >= 0, GenericError::AmountMustBePositive);
}

/// Rejects a flash-loan receiver that is not a deployed Wasm contract.
pub fn require_wasm_receiver(env: &Env, receiver: &Address) {
    assert_with_error!(
        env,
        matches!(receiver.executable(), Some(Executable::Wasm(_))),
        FlashLoanError::InvalidFlashloanReceiver
    );
}
